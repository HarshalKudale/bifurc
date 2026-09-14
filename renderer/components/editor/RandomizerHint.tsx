import React from "react";
import { RANDOMIZER_TOKENS } from "@/lib/randomizer";
import { strings } from "@/lib/strings";
import TokenHint from "@/components/editor/TokenHint";

interface Props {
  onInsert: (token: string) => void;
}

/**
 * Button that opens a popover listing all {{random.*}} tokens.
 * Clicking a token calls onInsert("{{random.xxx}}").
 */
export default function RandomizerHint({ onInsert }: Props) {
  return (
    <TokenHint
      title={strings.editor.insertRandomToken}
      triggerLabel="random"
      headerLabel={strings.editor.randomizerTokens}
      accentClass="text-violet"
      onInsert={onInsert}
      minWidthClass="min-w-[260px]"
      maxHeightClass="max-h-72"
      labelClassName=""
      items={RANDOMIZER_TOKENS.map((t) => ({ key: t.key, label: t.description }))}
    />
  );
}
