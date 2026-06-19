import { StatusLine } from "@/components/states";
import { Card, CardHeader, CardTitle } from "@/components/ui/card";

export default function Repositories() {
  return (
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-lg font-medium tracking-tight">Repositories</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Connected repositories and their indexing health.
        </p>
      </div>
      <Card>
        <CardHeader>
          <CardTitle>Indexing health</CardTitle>
        </CardHeader>
        <StatusLine>
          Repository index status (per ADR-0016) lands with the indexer. Until then, connect repos
          by installing the GitHub App — opened pull requests will start appearing under Runs.
        </StatusLine>
      </Card>
    </div>
  );
}
