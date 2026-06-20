//! Task execution: turn a claimed task into one Kubernetes Job (ADR-0004).
//!
//! The dispatcher owns *which* task runs and *when*; the actual (potentially untrusted) work runs in
//! an isolated, per-task Job with TTL cleanup and task-scoped credentials. [`TaskLauncher`] is the
//! seam: `KubeLauncher` creates real Jobs in production, while tests build the manifest and assert
//! its shape without a cluster.

use k8s_openapi::api::batch::v1::Job;
use kube::api::PostParams;
use kube::{Api, Client};
use serde_json::{json, Value};

use crate::db::ClaimedTask;

/// Launches the execution of a claimed task and returns the created Job's name.
#[allow(async_fn_in_trait)] // crate-internal trait; never used across crates or as `dyn`.
pub trait TaskLauncher {
    async fn launch(&self, task: &ClaimedTask) -> anyhow::Result<String>;
}

/// Creates one Kubernetes Job per task via the cluster API.
pub struct KubeLauncher {
    jobs: Api<Job>,
    image: String,
    service_account: String,
    /// In-cluster URL the runner calls back for context + status (the control plane's own Service).
    control_plane_url: String,
    /// Shared bearer the runner presents to that internal API (ADR-0017). Injected into the Job so
    /// the runner can authenticate; empty when unset (the internal API is then disabled anyway).
    runner_token: String,
    /// Secret (in the agents namespace) holding the internal CA (`ca.crt`) the runner must trust to
    /// reach the eaig gateway's HTTPS embeddings endpoint. `None` → no CA volume mounted.
    ca_secret: Option<String>,
}

impl KubeLauncher {
    /// Build from the ambient Kubernetes config (in-cluster service account, or local kubeconfig).
    pub async fn from_env() -> anyhow::Result<Self> {
        let client = Client::try_default().await?;
        let namespace =
            std::env::var("AGENT_NAMESPACE").unwrap_or_else(|_| "lightbridge-agents".to_string());
        let image = std::env::var("AGENT_RUNNER_IMAGE")
            .unwrap_or_else(|_| "ghcr.io/vymalo/lightbridge-agent-runner:latest".to_string());
        let service_account = std::env::var("AGENT_SERVICE_ACCOUNT")
            .unwrap_or_else(|_| "lightbridge-agent".to_string());
        // Where the runner calls back. Defaults to the in-cluster Service name the ai-helm chart
        // gives the serve role; override per environment with CONTROL_PLANE_INTERNAL_URL.
        let control_plane_url = std::env::var("CONTROL_PLANE_INTERNAL_URL")
            .unwrap_or_else(|_| "http://lightbridge-ci-control-plane:8080".to_string());
        let runner_token = std::env::var("AGENT_RUNNER_TOKEN").unwrap_or_default();
        let ca_secret = std::env::var("AGENT_CA_SECRET")
            .ok()
            .filter(|s| !s.is_empty());
        Ok(Self {
            jobs: Api::namespaced(client, &namespace),
            image,
            service_account,
            control_plane_url,
            runner_token,
            ca_secret,
        })
    }
}

impl TaskLauncher for KubeLauncher {
    async fn launch(&self, task: &ClaimedTask) -> anyhow::Result<String> {
        let name = job_name(task);
        let manifest = job_manifest(
            &name,
            &self.image,
            &self.service_account,
            &self.control_plane_url,
            &self.runner_token,
            self.ca_secret.as_deref(),
            task,
        );
        let job: Job = serde_json::from_value(manifest)?;
        match self.jobs.create(&PostParams::default(), &job).await {
            Ok(_) => Ok(name),
            // The Job name is derived from the unique task id, so a 409 means *our own* Job already
            // exists — e.g. a previous attempt created it but we crashed before recording job_name,
            // or the create timed out after the apiserver accepted it. Adopt it instead of erroring,
            // which would requeue and 409 forever (dispatch is at-least-once).
            Err(kube::Error::Api(error)) if error.code == 409 => {
                tracing::warn!(job_name = %name, task_id = %task.id, "job already exists; adopting it");
                Ok(name)
            }
            Err(error) => Err(error.into()),
        }
    }
}

/// Deterministic, collision-free Job name for a task (the task id is unique).
fn job_name(task: &ClaimedTask) -> String {
    format!("lightbridge-agent-{}", task.id)
}

/// The per-task agent Job manifest (mirrors docs/kubernetes-deployment.md): `restartPolicy: Never`,
/// a TTL for cleanup, an active deadline to bound runtime, a least-privilege service account, and the
/// task id passed through so the runner can fetch its work and report back.
fn job_manifest(
    name: &str,
    image: &str,
    service_account: &str,
    control_plane_url: &str,
    runner_token: &str,
    ca_secret: Option<&str>,
    task: &ClaimedTask,
) -> Value {
    // Pass the claimed task's context so the runner knows what to act on (target + SHAs) and how to
    // report back: the task id, plus where to call (CONTROL_PLANE_URL) and the shared bearer it
    // presents (AGENT_RUNNER_TOKEN). The runner fetches full context from the internal API rather
    // than trusting these env values for anything security-sensitive.
    let mut env = vec![
        json!({ "name": "TASK_ID", "value": task.id.to_string() }),
        json!({ "name": "REPOSITORY_ID", "value": task.repository_id.to_string() }),
        json!({ "name": "INSTALLATION_ID", "value": task.installation_id.to_string() }),
        json!({ "name": "COMMAND", "value": task.command_text }),
        json!({ "name": "TARGET_TYPE", "value": task.target_type }),
        json!({ "name": "TARGET_ID", "value": task.target_id.to_string() }),
        json!({ "name": "ATTEMPT", "value": task.attempts.to_string() }),
        json!({ "name": "CONTROL_PLANE_URL", "value": control_plane_url }),
        json!({ "name": "AGENT_RUNNER_TOKEN", "value": runner_token }),
        // Embeddings config (ADR-0018): all three are required — no defaults — so a misconfigured
        // Job fails loud rather than silently embedding with the wrong model.
        json!({ "name": "EMBEDDINGS_BASE_URL",
            "valueFrom": { "secretKeyRef": { "name": "lightbridge-agent-secrets", "key": "embeddings-base-url" } } }),
        json!({ "name": "EMBEDDINGS_API_KEY",
            "valueFrom": { "secretKeyRef": { "name": "lightbridge-agent-secrets", "key": "embeddings-api-key" } } }),
        json!({ "name": "EMBEDDINGS_MODEL",
            "valueFrom": { "secretKeyRef": { "name": "lightbridge-agent-secrets", "key": "embeddings-model" } } }),
        // Review LLM config (ADR-0021). `optional: true`: absent these keys, the secretKeyRef
        // resolves to unset (not a pod start failure), so `LLM_MODEL` is unset and the runner skips
        // the review step (indexing-only). The operator enables review by populating these keys.
        json!({ "name": "LLM_BASE_URL",
            "valueFrom": { "secretKeyRef": { "name": "lightbridge-agent-secrets", "key": "llm-base-url", "optional": true } } }),
        json!({ "name": "LLM_API_KEY",
            "valueFrom": { "secretKeyRef": { "name": "lightbridge-agent-secrets", "key": "llm-api-key", "optional": true } } }),
        json!({ "name": "LLM_MODEL",
            "valueFrom": { "secretKeyRef": { "name": "lightbridge-agent-secrets", "key": "llm-model", "optional": true } } }),
    ];
    if let Some(base_sha) = &task.base_sha {
        env.push(json!({ "name": "BASE_SHA", "value": base_sha }));
    }
    if let Some(head_sha) = &task.head_sha {
        env.push(json!({ "name": "HEAD_SHA", "value": head_sha }));
    }

    // Internal CA: when configured, mount the gateway's CA so the Job can verify the HTTPS eaig
    // endpoint (the gateway's cert is from a private issuer, ClusterIssuer/self-signed-ca).
    //   - `EMBEDDINGS_CA_CERT`: the runner's reqwest client adds it via `add_root_certificate`
    //     (ADR-0018).
    //   - `NODE_EXTRA_CA_CERTS`: OpenCode (slice 5, ADR-0021) is a Bun binary whose TLS list is
    //     bundled-roots + system + this env (verified). It must be a process env var (Bun freezes the
    //     CA list on first TLS use), so the agent inherits it from the Job rather than us setting it
    //     on the spawned process.
    let (volumes, volume_mounts) = match ca_secret {
        Some(secret) => {
            env.push(json!({ "name": "EMBEDDINGS_CA_CERT", "value": "/etc/internal-ca/ca.crt" }));
            env.push(json!({ "name": "NODE_EXTRA_CA_CERTS", "value": "/etc/internal-ca/ca.crt" }));
            (
                json!([{ "name": "internal-ca", "secret": { "secretName": secret } }]),
                json!([{ "name": "internal-ca", "mountPath": "/etc/internal-ca", "readOnly": true }]),
            )
        }
        None => (json!([]), json!([])),
    };

    json!({
        "apiVersion": "batch/v1",
        "kind": "Job",
        "metadata": {
            "name": name,
            "labels": {
                "app.kubernetes.io/name": "lightbridge",
                "app.kubernetes.io/component": "agent",
                "app.kubernetes.io/part-of": "lightbridge-platform",
                "app.kubernetes.io/managed-by": "control-plane",
                "lightbridge.dev/task-id": task.id.to_string(),
            }
        },
        "spec": {
            "backoffLimit": 1,
            // Runtime cap: clone + tree-sitter chunk + embed + Graphify can legitimately run for
            // tens of minutes on a large repo (observed >15min in prod purely on indexing), so bound
            // it at 1h rather than killing healthy long indexers. `ttlSecondsAfterFinished` below is a
            // separate, shorter post-completion cleanup window.
            "activeDeadlineSeconds": 3600,
            "ttlSecondsAfterFinished": 900,
            "template": {
                "metadata": {
                    "labels": {
                        "app.kubernetes.io/name": "lightbridge",
                        "app.kubernetes.io/component": "agent",
                    }
                },
                "spec": {
                    "serviceAccountName": service_account,
                    "restartPolicy": "Never",
                    "volumes": volumes,
                    "containers": [{
                        "name": "runner",
                        "image": image,
                        "env": env,
                        "volumeMounts": volume_mounts,
                    }]
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn sample_task() -> ClaimedTask {
        ClaimedTask {
            id: Uuid::nil(),
            repository_id: 1,
            installation_id: 42,
            target_type: "pull_request".to_string(),
            target_id: 7,
            command_text: "review".to_string(),
            base_sha: None,
            head_sha: Some("deadbeef".to_string()),
            attempts: 1,
        }
    }

    #[test]
    fn job_name_is_derived_from_the_task_id() {
        assert_eq!(
            job_name(&sample_task()),
            "lightbridge-agent-00000000-0000-0000-0000-000000000000"
        );
    }

    #[test]
    fn manifest_is_a_valid_job_with_task_wiring() {
        let task = sample_task();
        let value = job_manifest(
            &job_name(&task),
            "img:tag",
            "sa",
            "http://cp:8080",
            "runner-secret",
            Some("lightbridge-agent-ca"),
            &task,
        );

        // Deserializes into the typed k8s Job (catches structural mistakes without a cluster).
        let job: Job = serde_json::from_value(value.clone()).expect("valid Job manifest");
        let spec = job.spec.expect("job spec");
        let pod = spec.template.spec.expect("pod spec");
        assert_eq!(pod.restart_policy.as_deref(), Some("Never"));
        assert_eq!(spec.active_deadline_seconds, Some(3600));
        assert_eq!(spec.ttl_seconds_after_finished, Some(900));
        assert_eq!(pod.service_account_name.as_deref(), Some("sa"));

        let container = &pod.containers[0];
        assert_eq!(container.image.as_deref(), Some("img:tag"));
        let env = container.env.as_ref().expect("env");
        let value_of = |name: &str| {
            env.iter()
                .find(|e| e.name == name)
                .and_then(|e| e.value.clone())
        };
        assert_eq!(
            value_of("TASK_ID").as_deref(),
            Some(task.id.to_string().as_str())
        );
        assert_eq!(value_of("COMMAND").as_deref(), Some("review"));
        assert_eq!(value_of("HEAD_SHA").as_deref(), Some("deadbeef"));
        // base_sha is None on the sample, so BASE_SHA is omitted entirely.
        assert!(value_of("BASE_SHA").is_none());
        // The runner's callback wiring travels with the Job.
        assert_eq!(
            value_of("CONTROL_PLANE_URL").as_deref(),
            Some("http://cp:8080")
        );
        assert_eq!(
            value_of("AGENT_RUNNER_TOKEN").as_deref(),
            Some("runner-secret")
        );

        // Embeddings (required) + LLM (optional) config are injected from the agent secret.
        let env_ref = |name: &str| env.iter().find(|e| e.name == name);
        let secret_key = |name: &str| -> Option<(String, Option<bool>)> {
            let e = env_ref(name)?;
            let sel = e.value_from.as_ref()?.secret_key_ref.as_ref()?;
            Some((sel.key.clone(), sel.optional))
        };
        assert_eq!(
            secret_key("EMBEDDINGS_MODEL"),
            Some(("embeddings-model".to_string(), None)),
            "embeddings is required (not optional)"
        );
        assert_eq!(
            secret_key("LLM_MODEL"),
            Some(("llm-model".to_string(), Some(true))),
            "review LLM is optional so the Job starts without it"
        );

        // The internal CA is mounted and both the runner (reqwest) and OpenCode (Bun) are pointed
        // at it.
        assert_eq!(
            value_of("EMBEDDINGS_CA_CERT").as_deref(),
            Some("/etc/internal-ca/ca.crt")
        );
        assert_eq!(
            value_of("NODE_EXTRA_CA_CERTS").as_deref(),
            Some("/etc/internal-ca/ca.crt")
        );
        let mount = container
            .volume_mounts
            .as_ref()
            .and_then(|m| m.iter().find(|m| m.name == "internal-ca"))
            .expect("internal-ca volumeMount");
        assert_eq!(mount.mount_path, "/etc/internal-ca");
        let vol = pod
            .volumes
            .as_ref()
            .and_then(|v| v.iter().find(|v| v.name == "internal-ca"))
            .expect("internal-ca volume");
        assert_eq!(
            vol.secret.as_ref().and_then(|s| s.secret_name.as_deref()),
            Some("lightbridge-agent-ca")
        );
    }
}
