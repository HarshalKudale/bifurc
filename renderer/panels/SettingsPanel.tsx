import React from "react";
import { AppConfig } from "@/types";
import { strings } from "@/lib/strings";
import { ColorMode } from "@/hooks/useTheme";
import PanelLayout from "@/components/ui/PanelLayout";
import ServerSettingsSection from "./settings/ServerSettingsSection";
import TlsSettingsSection from "./settings/TlsSettingsSection";
import AppearanceSection from "./settings/AppearanceSection";
import AboutSection from "./settings/AboutSection";

interface SettingsPanelProps {
  config: AppConfig;
  serverRunning: boolean;
  serverError: string | null;
  onConfigChange: (cfg: AppConfig) => Promise<void>;
  onServerRestart: () => Promise<void>;
  colorMode: ColorMode;
  onColorModeChange: (m: ColorMode) => void;
  sidebarVisibility: Record<string, boolean>;
  onSidebarVisibilityChange: (id: string, visible: boolean) => void;
}

export default function SettingsPanel({
  config,
  serverRunning,
  serverError,
  onConfigChange,
  onServerRestart,
  colorMode,
  onColorModeChange,
  sidebarVisibility,
  onSidebarVisibilityChange,
}: SettingsPanelProps) {
  return (
    <PanelLayout title={strings.settings.title} subtitle={strings.settings.subtitle}>
      <div className="flex flex-col gap-6">
        <AboutSection />

        <ServerSettingsSection
          config={config}
          serverRunning={serverRunning}
          serverError={serverError}
          onConfigChange={onConfigChange}
          onServerRestart={onServerRestart}
          colorMode={colorMode}
          onColorModeChange={onColorModeChange}
        />

        <TlsSettingsSection
          config={config}
          onConfigChange={onConfigChange}
        />

        <AppearanceSection
          sidebarVisibility={sidebarVisibility}
          onSidebarVisibilityChange={onSidebarVisibilityChange}
        />
      </div>
    </PanelLayout>
  );
}
