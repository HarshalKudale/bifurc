import React from "react";
import { AppConfig, SettingsPatch } from "@/types";
import { strings } from "@/lib/strings";
import { ColorMode } from "@/hooks/useTheme";
import PanelLayout from "@/components/ui/PanelLayout";
import ServerSettingsSection from "./settings/ServerSettingsSection";
import PortsSettingsSection from "./settings/PortsSettingsSection";
import TlsSettingsSection from "./settings/TlsSettingsSection";
import AppearanceSection from "./settings/AppearanceSection";
import AboutSection from "./settings/AboutSection";

interface SettingsPanelProps {
  config: AppConfig;
  serverRunning: boolean;
  serverError: string | null;
  onSettingsChange: (patch: SettingsPatch) => Promise<void>;
  onServerRestart: () => Promise<void>;
  colorMode: ColorMode;
  onColorModeChange: (m: ColorMode) => void;
  sidebarVisibility: Record<string, boolean>;
  onSidebarVisibilityChange: (id: import("@/lib/panelRegistry").Panel, visible: boolean) => void;
}

export default function SettingsPanel({
  config,
  serverRunning,
  serverError,
  onSettingsChange,
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
          onSettingsChange={onSettingsChange}
          onServerRestart={onServerRestart}
          colorMode={colorMode}
          onColorModeChange={onColorModeChange}
        />

        {/* The four host-level ports, in one place (`plan/17` work item 6). */}
        <PortsSettingsSection config={config} onSettingsChange={onSettingsChange} />

        <TlsSettingsSection
          config={config}
          onSettingsChange={onSettingsChange}
        />

        <AppearanceSection
          sidebarVisibility={sidebarVisibility}
          onSidebarVisibilityChange={onSidebarVisibilityChange}
        />
      </div>
    </PanelLayout>
  );
}
