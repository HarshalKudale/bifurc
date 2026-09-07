import React, { useState } from "react";
import iconUrl from "@/icon.png";
import { strings } from "@/lib/strings";
import { SectionLabel, SectionCard } from "@/components/ui";
import { Star, Heart, RefreshCw, Download, ExternalLink, CheckCircle2, AlertCircle, X } from "@/lib/icons";
import { UpdateCheckResult } from "@/types";

export default function AboutSection() {
  const [checking, setChecking] = useState(false);
  const [updateResult, setUpdateResult] = useState<UpdateCheckResult | null>(null);

  const handleCheckUpdate = async () => {
    setChecking(true);
    try {
      const res = await window.api.checkUpdate();
      setUpdateResult(res);
    } catch (err: any) {
      setUpdateResult({
        ok: false,
        hasUpdate: false,
        currentVersion: __APP_VERSION__,
        latestVersion: __APP_VERSION__,
        downloadUrl: "https://github.com/HarshalKudale/local-panel/releases",
        releaseUrl: "https://github.com/HarshalKudale/local-panel/releases",
        error: err?.message || "Failed to check for updates",
      });
    } finally {
      setChecking(false);
    }
  };

  return (
    <section>
      <SectionLabel>{strings.settings.sectionAbout}</SectionLabel>

      <SectionCard>
        <div className="px-5 py-4 border-b border-border flex items-center gap-4">
          <img src={iconUrl} className="w-10 h-10 rounded-lg flex-shrink-0" alt={strings.titleBar.appName} />
          <div>
            <div className="text-sm font-semibold text-foreground">{strings.titleBar.appName}</div>
            <div className="text-xs text-muted-foreground mt-0.5">{strings.settings.appDesc}</div>
          </div>
        </div>
        <div className="px-5 py-4 flex flex-col gap-3 text-xs text-muted-foreground">
          <AboutRow label={strings.settings.version}>
            <span className="text-foreground font-mono">{__APP_VERSION__}</span>
          </AboutRow>
          <AboutRow label={strings.settings.author}>
            <span className="text-foreground">{strings.settings.authorName}</span>
          </AboutRow>
          <AboutRow label={strings.settings.github}>
            <a
              href={strings.settings.githubUrl}
              onClick={(e) => {
                e.preventDefault();
                window.api.openExternal(strings.settings.githubUrl);
              }}
              className="text-signal hover:underline font-mono cursor-pointer"
            >
              {strings.settings.githubDisplay}
            </a>
          </AboutRow>
          <AboutRow label={strings.settings.license}>
            <span className="text-foreground">{strings.settings.licenseName}</span>
          </AboutRow>
        </div>

        {/* Update checker status banner */}
        {updateResult && (
          <div className="px-5 pb-3">
            {updateResult.hasUpdate ? (
              <div className="p-4 rounded-lg bg-signal/10 border border-signal/30 flex flex-col gap-2.5 relative">
                <button
                  onClick={() => setUpdateResult(null)}
                  className="absolute top-3 right-3 text-muted-foreground hover:text-foreground cursor-pointer p-0.5"
                  title="Dismiss"
                >
                  <X size={13} />
                </button>
                <div className="flex items-center gap-2">
                  <span className="w-2 h-2 rounded-full bg-signal animate-pulse" />
                  <span className="text-xs font-semibold text-signal">
                    {strings.settings.updateAvailable}: {updateResult.latestVersion}
                  </span>
                  <span className="text-[10px] text-muted-foreground font-mono">
                    (current: v{__APP_VERSION__.replace(/^v/, "")})
                  </span>
                </div>

                <p className="text-xs text-foreground/90 pr-6">
                  {updateResult.releaseName
                    ? `${updateResult.releaseName} is available for download.`
                    : strings.settings.updateAvailableDesc.replace("{version}", updateResult.latestVersion)}
                </p>

                <div className="flex flex-wrap items-center gap-2 pt-1">
                  <button
                    onClick={() => window.api.openExternal(updateResult.downloadUrl)}
                    className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md bg-signal text-background text-xs font-semibold hover:opacity-90 transition-opacity cursor-pointer shadow-sm"
                  >
                    <Download size={13} />
                    {updateResult.assetName ? `Download ${updateResult.assetName}` : strings.settings.downloadUpdate}
                  </button>

                  {updateResult.releaseUrl && (
                    <button
                      onClick={() => window.api.openExternal(updateResult.releaseUrl)}
                      className="inline-flex items-center gap-1 px-3 py-1.5 rounded-md border border-border text-xs text-muted-foreground hover:text-foreground hover:bg-surface-2 transition-colors cursor-pointer"
                    >
                      <ExternalLink size={12} />
                      {strings.settings.viewReleaseNotes}
                    </button>
                  )}
                </div>
              </div>
            ) : updateResult.ok ? (
              <div className="p-3 rounded-lg bg-surface border border-border/80 flex items-center justify-between gap-2.5 text-xs text-muted-foreground">
                <div className="flex items-center gap-2">
                  <CheckCircle2 size={15} className="text-signal flex-shrink-0" />
                  <span>
                    {strings.settings.upToDateDesc.replace("{version}", updateResult.latestVersion || `v${__APP_VERSION__}`)}
                  </span>
                </div>
                <button
                  onClick={() => setUpdateResult(null)}
                  className="text-muted-foreground hover:text-foreground cursor-pointer p-0.5"
                  title="Dismiss"
                >
                  <X size={13} />
                </button>
              </div>
            ) : (
              <div className="p-3 rounded-lg bg-destructive/10 border border-destructive/20 flex items-center justify-between gap-2.5 text-xs text-destructive">
                <div className="flex items-center gap-2">
                  <AlertCircle size={15} className="flex-shrink-0" />
                  <span>{updateResult.error || strings.settings.updateCheckFailed}</span>
                </div>
                <button
                  onClick={() => setUpdateResult(null)}
                  className="text-destructive/70 hover:text-destructive cursor-pointer p-0.5"
                  title="Dismiss"
                >
                  <X size={13} />
                </button>
              </div>
            )}
          </div>
        )}

        <div className="px-5 py-4 border-t border-border flex flex-wrap gap-3">
          <button
            onClick={() => window.api.openExternal("https://github.com/HarshalKudale/local-panel")}
            className="inline-flex items-center gap-2 px-3 py-1.5 rounded-md bg-surface-2 border border-border text-xs font-medium text-foreground hover:border-signal/50 hover:text-signal transition-colors cursor-pointer"
          >
            <svg width="14" height="14" viewBox="0 0 16 16" fill="currentColor">
              <path d="M8 .2a8 8 0 0 0-2.53 15.59c.4.07.55-.17.55-.38l-.01-1.49c-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82a7.42 7.42 0 0 1 4 0c1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48l-.01 2.2c0 .21.15.46.55.38A8.01 8.01 0 0 0 8 .2z" />
            </svg>
            <Star size={14} /> {strings.settings.starOnGitHub}
          </button>
          <button
            onClick={() =>
              window.api.openExternal("https://github.com/HarshalKudale/local-panel/blob/main/SUPPORT.md")
            }
            className="inline-flex items-center gap-2 px-3 py-1.5 rounded-md bg-surface-2 border border-border text-xs font-medium text-foreground hover:border-signal/50 hover:text-signal transition-colors cursor-pointer"
          >
            <Heart size={14} /> {strings.settings.supportProject}
          </button>
          <button
            onClick={handleCheckUpdate}
            disabled={checking}
            className="inline-flex items-center gap-2 px-3 py-1.5 rounded-md bg-surface-2 border border-border text-xs font-medium text-foreground hover:border-signal/50 hover:text-signal transition-colors cursor-pointer disabled:opacity-60"
          >
            <RefreshCw size={14} className={checking ? "animate-spin text-signal" : ""} />
            {checking ? strings.settings.checkingForUpdates : strings.settings.checkForUpdates}
          </button>
        </div>
      </SectionCard>
    </section>
  );
}

function AboutRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-center gap-3">
      <span className="text-foreground font-medium w-16 flex-shrink-0">{label}</span>
      {children}
    </div>
  );
}
