import { HealthBarService } from "@/types";

export interface HealthCheckResult {
  ok: boolean;
  statusCode: number | null;
  body: string | null;
  headers: Record<string, string> | null;
  error: string | null;
  durationMs: number;
}

export type CheckStatus = "idle" | "checking" | "success" | "error";

export interface ServiceState {
  status: CheckStatus;
  statusCode: number | null;
  body: string | null;
  headers: Record<string, string> | null;
  error: string | null;
  durationMs: number | null;
  checkedAt: number | null;
}
