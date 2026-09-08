import React, { useState } from "react";
import { AppConfig } from "@/types";
import { strings } from "@/lib/strings";
import { Button, SectionLabel, SectionCard, SettingsRow, Switch } from "@/components/ui";

interface TlsSettingsSectionProps {
  config: AppConfig;
  onConfigChange: (cfg: AppConfig) => Promise<void>;
}

export default function TlsSettingsSection({ config, onConfigChange }: TlsSettingsSectionProps) {
  const [tlsImporting, setTlsImporting] = useState<"cert" | "key" | null>(null);
  const [tlsGenerating, setTlsGenerating] = useState(false);
  const [tlsInstalling, setTlsInstalling] = useState(false);
  const [tlsInstallResult, setTlsInstallResult] = useState<{
    ok: boolean;
    needsManualInstall?: boolean;
    instructions?: string;
    error?: string;
  } | null>(null);

  const handleGlobalChange = (
    patch: Partial<Pick<AppConfig, "tlsEnabled" | "tlsCaCertPath" | "tlsCaKeyPath">>
  ) => {
    onConfigChange({ ...config, ...patch });
  };

  const hasCert = !!config.tlsCaCertPath;
  const hasKey = !!config.tlsCaKeyPath;
  const tlsReady = hasCert && hasKey;

  const handleImportCert = async () => {
    setTlsImporting("cert");
    try {
      const result = await window.api.tlsImportCert();
      if (result.ok && result.path) {
        handleGlobalChange({ tlsCaCertPath: result.path });
      }
    } finally {
      setTlsImporting(null);
    }
  };

  const handleImportKey = async () => {
    setTlsImporting("key");
    try {
      const result = await window.api.tlsImportKey();
      if (result.ok && result.path) {
        handleGlobalChange({ tlsCaKeyPath: result.path });
      }
    } finally {
      setTlsImporting(null);
    }
  };

  const handleRemoveCert = async () => {
    await window.api.tlsRemoveCert();
    setTlsInstallResult(null);
    handleGlobalChange({ tlsEnabled: false, tlsCaCertPath: null, tlsCaKeyPath: null });
  };

  const handleGenerateCert = async () => {
    setTlsGenerating(true);
    setTlsInstallResult(null);
    try {
      const result = await window.api.tlsGenerate();
      if (result.ok && result.certPath && result.keyPath) {
        handleGlobalChange({ tlsCaCertPath: result.certPath, tlsCaKeyPath: result.keyPath });
      }
    } finally {
      setTlsGenerating(false);
    }
  };

  const handleInstallCA = async () => {
    setTlsInstalling(true);
    setTlsInstallResult(null);
    try {
      const result = await window.api.tlsInstallCA();
      setTlsInstallResult(result);
    } finally {
      setTlsInstalling(false);
    }
  };

  const handleExportCert = async () => {
    await window.api.tlsExportCert();
  };

  return (
    <section>
      <SectionLabel>{strings.settings.sectionTls}</SectionLabel>

      <div className="mb-3 p-4 rounded border border-signal/20 bg-signal/5 text-xs text-muted-foreground leading-relaxed">
        <p>{strings.settings.tlsGenerateHint}</p>
      </div>

      <SectionCard>
        <SettingsRow title={strings.settings.tlsCaCert} desc={strings.settings.tlsCaCertDesc}>
          <div className="flex items-center gap-2">
            <span
              className={`text-xs font-mono truncate max-w-[160px] ${
                hasCert ? "text-signal" : "text-muted-foreground"
              }`}
            >
              {hasCert
                ? config.tlsCaCertPath!.split(/[\\/]/).pop()
                : strings.settings.tlsNotConfigured}
            </span>
            <Button
              variant="primary"
              onClick={handleGenerateCert}
              disabled={tlsGenerating || tlsImporting !== null}
            >
              {tlsGenerating ? strings.settings.tlsGenerating : strings.settings.tlsGenerate}
            </Button>
            <Button
              variant="secondary"
              onClick={handleImportCert}
              disabled={tlsGenerating || tlsImporting !== null}
            >
              {tlsImporting === "cert" ? "…" : strings.settings.tlsImport}
            </Button>
          </div>
        </SettingsRow>

        <SettingsRow title={strings.settings.tlsCaKey} desc={strings.settings.tlsCaKeyDesc}>
          <div className="flex items-center gap-2">
            <span
              className={`text-xs font-mono ${hasKey ? "text-signal" : "text-muted-foreground"}`}
            >
              {hasKey
                ? config.tlsCaKeyPath!.split(/[\\/]/).pop()
                : strings.settings.tlsNotConfigured}
            </span>
            <Button
              variant="secondary"
              onClick={handleImportKey}
              disabled={tlsGenerating || tlsImporting !== null}
            >
              {tlsImporting === "key" ? "…" : strings.settings.tlsImport}
            </Button>
          </div>
        </SettingsRow>

        {tlsReady && (
          <SettingsRow
            title={strings.settings.tlsInstallCA}
            desc={strings.settings.tlsInstallHint}
          >
            <div className="flex flex-col items-end gap-2">
              <div className="flex items-center gap-2">
                <Button
                  variant="secondary"
                  onClick={handleInstallCA}
                  disabled={tlsInstalling}
                >
                  {tlsInstalling ? strings.settings.tlsInstalling : strings.settings.tlsInstallCA}
                </Button>
                <Button variant="secondary" onClick={handleExportCert}>
                  {strings.settings.tlsExportCert}
                </Button>
                <Button variant="secondary" onClick={handleRemoveCert}>
                  {strings.settings.tlsRemove}
                </Button>
              </div>

              {tlsInstallResult && (
                <div
                  className={`text-xs px-3 py-2 rounded border w-full ${
                    tlsInstallResult.ok
                      ? "border-signal/30 bg-signal/5 text-signal"
                      : "border-destructive/30 bg-destructive/5 text-destructive"
                  }`}
                >
                  {tlsInstallResult.ok ? (
                    strings.settings.tlsInstallSuccess
                  ) : tlsInstallResult.needsManualInstall ? (
                    <>
                      <p className="mb-1 font-medium">{strings.settings.tlsManualInstall}</p>
                      <pre className="font-mono text-[10px] whitespace-pre-wrap break-all text-muted-foreground">
                        {tlsInstallResult.instructions}
                      </pre>
                    </>
                  ) : (
                    `${strings.settings.tlsInstallFailed}: ${tlsInstallResult.error}`
                  )}
                </div>
              )}
            </div>
          </SettingsRow>
        )}

        <SettingsRow title={strings.settings.tlsEnabled} desc={strings.settings.tlsEnabledDesc}>
          <Switch
            checked={config.tlsEnabled}
            ariaLabel={strings.settings.tlsEnabled}
            onChange={(v) => handleGlobalChange({ tlsEnabled: v })}
            disabled={!tlsReady}
          />
        </SettingsRow>
      </SectionCard>
    </section>
  );
}
