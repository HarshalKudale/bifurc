import React, { useState, useEffect } from "react";
import { AppConfig } from "@/types";
import { strings } from "@/lib/strings";
import { ColorMode } from "@/hooks/useTheme";
import { Button, Input, SectionLabel, SectionCard, SettingsRow, Switch } from "@/components/ui";

interface ServerSettingsSectionProps {
  config: AppConfig;
  serverRunning: boolean;
  serverError: string | null;
  onConfigChange: (cfg: AppConfig) => Promise<void>;
  onServerRestart: () => Promise<void>;
  colorMode: ColorMode;
  onColorModeChange: (m: ColorMode) => void;
}

export default function ServerSettingsSection({
  config,
  serverRunning,
  serverError,
  onConfigChange,
  onServerRestart,
  colorMode,
  onColorModeChange,
}: ServerSettingsSectionProps) {
  const [portInput, setPortInput] = useState(String(config.port));
  const [webhookPortInput, setWebhookPortInput] = useState(String(config.webhookPort ?? 9101));
  const [companionPortInput, setCompanionPortInput] = useState(String(config.companionPort ?? 9271));
  const [restarting, setRestarting] = useState(false);

  useEffect(() => {
    setPortInput(String(config.port));
    setWebhookPortInput(String(config.webhookPort ?? 9101));
    setCompanionPortInput(String(config.companionPort ?? 9271));
  }, [config.port, config.webhookPort, config.companionPort]);

  const handleGlobalChange = (
    patch: Partial<Pick<AppConfig, "port" | "minimizeToTray" | "webhookPort" | "companionPort">>
  ) => {
    onConfigChange({ ...config, ...patch });
  };

  const handlePortCommit = () => {
    const port = parseInt(portInput, 10);
    if (port >= 1 && port <= 65535) {
      if (port !== config.port) handleGlobalChange({ port });
    } else {
      setPortInput(String(config.port));
    }
  };

  const handleWebhookPortCommit = () => {
    const port = parseInt(webhookPortInput, 10);
    if (port >= 1 && port <= 65535) {
      if (port !== (config.webhookPort ?? 9101)) handleGlobalChange({ webhookPort: port });
    } else {
      setWebhookPortInput(String(config.webhookPort ?? 9101));
    }
  };

  const handleCompanionPortCommit = () => {
    const port = parseInt(companionPortInput, 10);
    if (port >= 1 && port <= 65535) {
      if (port !== (config.companionPort ?? 9271)) handleGlobalChange({ companionPort: port });
    } else {
      setCompanionPortInput(String(config.companionPort ?? 9271));
    }
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
          Optionally set <code className="font-mono bg-surface-2 px-1 py-0.5 rounded text-foreground">127.0.0.1:{config.port}</code> as your HTTP proxy. Proxy rules then apply to all HTTP traffic.{" "}
          {config.tlsEnabled ? strings.settings.httpsIntercepted : strings.settings.httpsTunnel}
        </p>
      </div>

      <SectionCard>
        <SettingsRow
          title={strings.settings.serverPort}
          desc={strings.settings.serverPortDesc.replace("{hint}", portHint)}
        >
          <Input
            aria-label={strings.settings.serverPort}
            type="number"
            className="w-24 text-right font-mono"
            min={1}
            max={65535}
            value={portInput}
            onChange={(e) => setPortInput(e.target.value)}
            onBlur={handlePortCommit}
            onKeyDown={(e) => { if (e.key === "Enter") handlePortCommit(); }}
          />
        </SettingsRow>

        <SettingsRow
          title={strings.settings.webhookServerPort}
          desc={`${strings.settings.webhookServerPortDesc} ${strings.settings.currentlyLabel} localhost:${config.webhookPort ?? 9101}/bifurc/webhooks/`}
        >
          <Input
            aria-label={strings.settings.webhookServerPort}
            type="number"
            className="w-24 text-right font-mono"
            min={1}
            max={65535}
            value={webhookPortInput}
            onChange={(e) => setWebhookPortInput(e.target.value)}
            onBlur={handleWebhookPortCommit}
            onKeyDown={(e) => { if (e.key === "Enter") handleWebhookPortCommit(); }}
          />
        </SettingsRow>

        <SettingsRow
          title={strings.settings.companionPort}
          desc={strings.settings.companionPortDesc}
        >
          <Input
            aria-label={strings.settings.companionPort}
            type="number"
            className="w-24 text-right font-mono"
            min={1}
            max={65535}
            value={companionPortInput}
            onChange={(e) => setCompanionPortInput(e.target.value)}
            onBlur={handleCompanionPortCommit}
            onKeyDown={(e) => { if (e.key === "Enter") handleCompanionPortCommit(); }}
          />
        </SettingsRow>

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
