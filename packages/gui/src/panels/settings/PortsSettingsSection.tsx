import React, { useState, useEffect } from "react";
import { AppConfig, SettingsPatch } from "@/types";
import { strings } from "@/lib/strings";
import { Badge, Input, SectionCard, SectionLabel, SettingsRow } from "@/components/ui";

interface PortsSettingsSectionProps {
  config: AppConfig;
  onSettingsChange: (patch: SettingsPatch) => Promise<void>;
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
 * re-bound at runtime by `settings.update`; the companion is not, so its row carries a badge rather than
 * the app silently keeping the old port. The badge is **static** rather than appearing only once the value has
 * changed, because the port the *running* companion is on is not on the wire: `companionPort` appears in no
 * protocol command — `serverStatus` is the edge's — so "this differs from the running one" would be a claim
 * the GUI cannot verify. It becomes a conditional badge the day a command reports the live port, and not
 * before.
 *
 * **There is no GUI port, so there is no GUI row.** The web UI host was decommissioned — the desktop app is the
 * single build target — so nothing serves a separate browser bundle and there is no companion-adjacent port to
 * state. `plan/17` §1's pair (companion plus a GUI neighbour) is gone; the companion stands alone.
 */
export default function PortsSettingsSection({ config, onSettingsChange }: PortsSettingsSectionProps) {
  const edge = usePortDraft(config.port, (port) => onSettingsChange({ port }));
  const forward = usePortDraft(config.forwardProxyPort, (port) =>
    onSettingsChange({ forwardProxyPort: port })
  );
  const companion = usePortDraft(config.companionPort, (port) =>
    onSettingsChange({ companionPort: port })
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
    </section>
  );
}
