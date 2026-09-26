import React, { useState, useEffect } from "react";
import { AppConfig } from "@/types";
import { strings } from "@/lib/strings";
import { Badge, Input, SectionCard, SectionLabel, SettingsRow } from "@/components/ui";

interface PortsSettingsSectionProps {
  config: AppConfig;
  onConfigChange: (cfg: AppConfig) => Promise<void>;
}

/**
 * One port input's local draft.
 *
 * The draft is a **string**, not a number: a half-typed port (`""`, `"8"`) has to stay on screen while it is
 * being typed, and a `number` state would coerce it away under the user. It commits on blur or Enter, and
 * reverts to the saved value when what is in the box is not a port.
 */
function usePortDraft(saved: number, commit: (port: number) => void) {
  const [draft, setDraft] = useState(String(saved));

  useEffect(() => {
    setDraft(String(saved));
  }, [saved]);

  const onCommit = () => {
    const port = parseInt(draft, 10);
    if (port >= 1 && port <= 65535) {
      if (port !== saved) commit(port);
    } else {
      setDraft(String(saved));
    }
  };

  return { draft, setDraft, onCommit };
}

/**
 * Every port the host listens on, in one place — the whole point of `plan/17` work item 6.
 *
 * All four are **host-level**: a workspace is a project, so switching one changes which rules, mocks and
 * webhook endpoints are in force, not which ports the infrastructure is on. The section exists because the
 * four have four *different* consumers — the address bar, the browser's proxy setting, whoever sends the
 * webhooks, and the extension — and before it one of them was documented wrongly: the Server section told the
 * user to point their browser's proxy at the **edge** port, which is the reverse proxy's.
 *
 * **Only the companion needs a restart** (`plan/17` §1). The edge, the forward proxy and the webhook are
 * re-bound at runtime by `config.save` (item 7); the companion is not, so its row carries a badge rather than
 * the app silently keeping the old port. The badge is **static** rather than appearing only once the value has
 * changed, because the port the *running* companion is on is not on the wire: `companionPort` appears in no
 * protocol command — `serverStatus` is the edge's — so "this differs from the running one" would be a claim
 * the GUI cannot verify. It becomes a conditional badge the day a command reports the live port, and not
 * before.
 *
 * **The web GUI has no row and no input.** Its port is derived as `companionPort + 1` and handed to the GUI at
 * start, so an independently settable GUI port would be a way to configure the pair into a state where the GUI
 * looks for the companion somewhere it is not. It is stated in the note below rather than shown as a number,
 * for two reasons: the arithmetic lives in the engine (`webGuiPortFor`), a Node module the browser bundle must
 * not pull in; and **Electron has no GUI port at all** — only the web host does — so a number here would be
 * wrong in the desktop app, where this same panel renders.
 */
export default function PortsSettingsSection({ config, onConfigChange }: PortsSettingsSectionProps) {
  const edge = usePortDraft(config.port, (port) => onConfigChange({ ...config, port }));
  const forward = usePortDraft(config.forwardProxyPort, (port) =>
    onConfigChange({ ...config, forwardProxyPort: port })
  );
  const webhook = usePortDraft(config.webhookPort, (port) => onConfigChange({ ...config, webhookPort: port }));
  const companion = usePortDraft(config.companionPort, (port) =>
    onConfigChange({ ...config, companionPort: port })
  );

  const portHint = config.port === 80 ? "example.localhost" : `example.localhost:${config.port}`;

  /**
   * A render helper, **called** rather than used as `<PortInput …/>`.
   *
   * A nested component would be a new function identity on every render, so React would unmount and remount
   * the `<input>` each time — losing focus and the caret mid-edit. Calling a plain function keeps the element
   * type stable.
   */
  const renderPortInput = (
    draft: string,
    setDraft: (v: string) => void,
    commit: () => void,
    label: string
  ) => (
    <Input
      aria-label={label}
      type="number"
      className="w-24 text-right font-mono"
      min={1}
      max={65535}
      value={draft}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => { if (e.key === "Enter") commit(); }}
    />
  );

  return (
    <section>
      <SectionLabel>{strings.settings.sectionPorts}</SectionLabel>

      <SectionCard>
        <SettingsRow
          title={strings.settings.edgePort}
          desc={strings.settings.edgePortDesc.replace("{hint}", portHint)}
        >
          {renderPortInput(edge.draft, edge.setDraft, edge.onCommit, strings.settings.edgePort)}
        </SettingsRow>

        <SettingsRow
          title={strings.settings.forwardProxyPort}
          desc={strings.settings.forwardProxyPortDesc}
        >
          {renderPortInput(forward.draft, forward.setDraft, forward.onCommit, strings.settings.forwardProxyPort)}
        </SettingsRow>

        <SettingsRow
          title={strings.settings.webhookServerPort}
          desc={`${strings.settings.webhookServerPortDesc} ${strings.settings.currentlyLabel} localhost:${config.webhookPort}/bifurc/webhooks/`}
        >
          {renderPortInput(webhook.draft, webhook.setDraft, webhook.onCommit, strings.settings.webhookServerPort)}
        </SettingsRow>

        <SettingsRow
          title={strings.settings.companionPort}
          desc={strings.settings.companionPortDesc}
        >
          <div className="flex items-center gap-2">
            {renderPortInput(
              companion.draft,
              companion.setDraft,
              companion.onCommit,
              strings.settings.companionPort
            )}
            <Badge variant="yellow">{strings.settings.restartRequired}</Badge>
          </div>
        </SettingsRow>
      </SectionCard>

      <div className="mt-3 px-4 py-3 rounded border border-border bg-surface-2/40 text-xs text-muted-foreground leading-relaxed">
        {strings.settings.portsDerivedNote}
      </div>
    </section>
  );
}
