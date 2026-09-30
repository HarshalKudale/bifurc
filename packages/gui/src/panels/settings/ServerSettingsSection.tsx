import React, { useState } from "react";
import { AppConfig, SettingsPatch } from "@/types";
import { strings } from "@/lib/strings";
import { ColorMode } from "@/hooks/useTheme";
import { Button, SectionLabel, SectionCard, SettingsRow, Switch } from "@/components/ui";

interface ServerSettingsSectionProps {
  config: AppConfig;
  serverRunning: boolean;
  serverError: string | null;
  onSettingsChange: (patch: SettingsPatch) => Promise<void>;
  onServerRestart: () => Promise<void>;
  colorMode: ColorMode;
  onColorModeChange: (m: ColorMode) => void;
}

/**
 * The server as a **process**: its status, its lifecycle, and window behaviour.
 *
 * The four ports it listens on moved to `PortsSettingsSection` in `plan/17` work item 6. A port is host-level
 * infrastructure with one specific consumer, so the four belong side by side; what is left here is everything
 * that is not a port.
 */
export default function ServerSettingsSection({
  config,
  serverRunning,
  serverError,
  onSettingsChange,
  onServerRestart,
  colorMode,
  onColorModeChange,
}: ServerSettingsSectionProps) {
  const [restarting, setRestarting] = useState(false);

  const handleGlobalChange = (patch: Pick<SettingsPatch, "minimizeToTray">) => {
    onSettingsChange(patch);
  };

  const handleRestart = async () => {
    setRestarting(true);
    try {
      await onServerRestart();
    } finally {
      setRestarting(false);
    }
  };

  const portHint = config.port === 80 ? "example.localhost" : `example.localhost:${config.port}`;

  return (
    <section>
      <SectionLabel>{strings.settings.sectionServer}</SectionLabel>

      {serverError ? (
        <div className="mb-3 px-3 py-2 rounded border border-destructive/30 bg-destructive/5 text-xs text-destructive flex items-start gap-2">
          <span className="w-1.5 h-1.5 rounded-full bg-destructive flex-shrink-0 mt-0.5" />
          <span className="flex-1">{strings.server.error.replace("{error}", serverError)}</span>
        </div>
      ) : (
        <div className="mb-3 px-3 py-2 rounded border border-signal/20 bg-signal/5 text-xs text-signal flex items-center gap-2">
          <span
            className="w-1.5 h-1.5 rounded-full bg-signal flex-shrink-0"
            style={{ boxShadow: "var(--glow-signal-sm)" }}
          />
          {serverRunning
            ? strings.server.running.replace("{port}", String(config.port)).replace("{hint}", portHint)
            : strings.server.notRunning}
        </div>
      )}

      <div className="mb-3 p-4 rounded border border-signal/20 bg-signal/5 text-xs text-muted-foreground leading-relaxed flex flex-col gap-2">
        <p>
          <span className="text-signal font-semibold">{strings.settings.port80Hint}</span>{" "}
          Use <code className="font-mono bg-surface-2 px-1 py-0.5 rounded text-foreground">{strings.settings.port80Example}</code> — browsers resolve <code className="font-mono bg-surface-2 px-1 py-0.5 rounded text-foreground">*.localhost</code> to 127.0.0.1 automatically (RFC 6761).
        </p>
        <p>
          <span className="text-signal font-semibold">{strings.settings.otherPortHint}</span>{" "}
          Use <code className="font-mono bg-surface-2 px-1 py-0.5 rounded text-foreground">{portHint}</code> in the browser. No proxy setup needed.
        </p>
        <p>
          <span className="text-signal font-semibold">{strings.settings.proxyHint}</span>{" "}
          {/* The **forward proxy** port, not `config.port`. Those were the same number before `plan/17` split
              the reverse proxy (the edge) from the forward proxy, and this line kept naming the edge — which is
              the port `*.localhost` mappings point at, not the one a browser's proxy setting wants. */}
          Optionally set <code className="font-mono bg-surface-2 px-1 py-0.5 rounded text-foreground">127.0.0.1:{config.forwardProxyPort}</code> as your HTTP proxy. Proxy rules then apply to all HTTP traffic.{" "}
          {config.tlsEnabled ? strings.settings.httpsIntercepted : strings.settings.httpsTunnel}
        </p>
      </div>

      <SectionCard>
        <SettingsRow
          title={strings.settings.minimizeToTray}
          desc={strings.settings.minimizeToTrayDesc}
        >
          <Switch
            checked={config.minimizeToTray}
            ariaLabel={strings.settings.minimizeToTray}
            onChange={(v) => handleGlobalChange({ minimizeToTray: v })}
          />
        </SettingsRow>

        <SettingsRow title={strings.settings.restartServer} desc={strings.settings.restartServerDesc}>
          <Button
            variant="secondary"
            onClick={handleRestart}
            disabled={restarting}
          >
            {restarting ? strings.server.restarting : strings.server.restart}
          </Button>
        </SettingsRow>

        <SettingsRow title="Dark Mode" desc="Enable cyberpunk developer tool dark theme (defaults to dark)">
          <Switch
            checked={colorMode === "dark"}
            ariaLabel="Dark Mode"
            onChange={(dark) => onColorModeChange(dark ? "dark" : "light")}
          />
        </SettingsRow>
      </SectionCard>
    </section>
  );
}
