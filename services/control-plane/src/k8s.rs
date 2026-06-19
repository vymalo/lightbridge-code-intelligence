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
        Ok(Self {
            jobs: Api::namespaced(client, &namespace),
            image,
            service_account,
        })
    }
}

impl TaskLauncher for KubeLauncher {
    async fn launch(&self, task: &ClaimedTask) -> anyhow::Result<String> {
        let name = job_name(task);
        let manifest = job_manifest(&name, &self.image, &self.service_account, task);
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
fn job_manifest(name: &str, image: &str, service_account: &str, task: &ClaimedTask) -> Value {
    // Pass the claimed task's context so the runner knows what to act on (target + SHAs) and how to
    // report back (task / repository / installation ids), without an extra round-trip.
    let mut env = vec![
        json!({ "name": "TASK_ID", "value": task.id.to_string() }),
        json!({ "name": "REPOSITORY_ID", "value": task.repository_id.to_string() }),
        json!({ "name": "INSTALLATION_ID", "value": task.installation_id.to_string() }),
        json!({ "name": "COMMAND", "value": task.command_text }),
        json!({ "name": "TARGET_TYPE", "value": task.target_type }),
        json!({ "name": "TARGET_ID", "value": task.target_id.to_string() }),
        json!({ "name": "ATTEMPT", "value": task.attempts.to_string() }),
    ];
    if let Some(base_sha) = &task.base_sha {
        env.push(json!({ "name": "BASE_SHA", "value": base_sha }));
    }
    if let Some(head_sha) = &task.head_sha {
        env.push(json!({ "name": "HEAD_SHA", "value": head_sha }));
    }

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
            "activeDeadlineSeconds": 900,
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
                    "containers": [{
                        "name": "runner",
                        "image": image,
                        "env": env,
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
        let value = job_manifest(&job_name(&task), "img:tag", "sa", &task);

        // Deserializes into the typed k8s Job (catches structural mistakes without a cluster).
        let job: Job = serde_json::from_value(value.clone()).expect("valid Job manifest");
        let spec = job.spec.expect("job spec");
        let pod = spec.template.spec.expect("pod spec");
        assert_eq!(pod.restart_policy.as_deref(), Some("Never"));
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
    }
}
