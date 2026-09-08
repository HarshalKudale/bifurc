import React from "react";
import { Environment } from "@/types";
import { strings } from "@/lib/strings";
import TokenHint from "@/components/editor/TokenHint";

interface Props {
  env: Environment | null;
  onInsert: (token: string) => void;
}

/**
 * Small button that opens a popover listing available variables.
 * Clicking a variable calls onInsert("{{KEY}}") so the caller can
 * append it to whichever input is focused.
 */
export default function EnvVarHint({ env, onInsert }: Props) {
  if (!env || env.variables.length === 0) return null;

  return (
    <TokenHint
      title={strings.editor.activeEnv.replace("{name}", env.name)}
      triggerLabel={env.name.slice(0, 10)}
      headerLabel={strings.editor.variablesHeader.replace("{name}", env.name)}
      accentClass="text-signal"
      onInsert={onInsert}
      items={env.variables.filter((v) => v.key.trim()).map((v) => ({
        key: v.key,
        label: v.value || <em>{strings.editor.emptyValue}</em>,
      }))}
    />
  );
}
