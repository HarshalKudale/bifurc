// UI string constants for localization support — split by domain (see Cleanup_plan.md Phase 4)
import { navigationStrings } from "./navigation";
import { commonStrings } from "./common";
import { entitiesStrings } from "./entities";
import { protocolsStrings } from "./protocols";
import { workspaceStrings } from "./workspace";
import { settingsGroupStrings } from "./settingsMisc";
import { editorStrings } from "./editor";
import { sidebarStrings } from "./sidebar";

export const strings = {
  ...navigationStrings,
  ...commonStrings,
  ...entitiesStrings,
  ...protocolsStrings,
  ...workspaceStrings,
  ...settingsGroupStrings,
  ...editorStrings,
  ...sidebarStrings,
} as const;
