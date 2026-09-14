export interface GateResult {
    allowed: boolean;
    current?: number;
    limit?: number;
}

/**
 * Check whether a new entity of the given kind can be created in the workspace.
 * Returns { allowed: true } when no limits are enforced.
 */
export function gateCreate(_wsId: string, _kind: string): GateResult {
    return { allowed: true };
}

/**
 * Check whether an entity of the given kind can be enabled in the workspace.
 * Returns { allowed: true } when no limits are enforced.
 */
export function gateEnable(_wsId: string, _kind: string): GateResult {
    return { allowed: true };
}
