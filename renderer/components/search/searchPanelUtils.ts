import { Panel } from "@/lib/panelRegistry";

/** Check if a panel is tab-based */
export function isTabSupportedPanel(panel: Panel): boolean {
  return (
    panel === "requests" ||
    panel === "req-rest" ||
    panel === "req-graphql" ||
    panel === "req-soap" ||
    panel === "req-grpc" ||
    panel === "mocks" ||
    panel === "mock-rest" ||
    panel === "mock-graphql" ||
    panel === "mock-soap" ||
    panel === "mock-grpc" ||
    panel === "sockets" ||
    panel === "webhooks"
  );
}

/** Normalize panel id to base panel */
export function normalizePanel(panel: Panel): Panel {
  if (panel === "req-rest" || panel === "req-graphql" || panel === "req-soap" || panel === "req-grpc") {
    return "requests";
  }
  if (panel === "mock-rest" || panel === "mock-graphql" || panel === "mock-soap" || panel === "mock-grpc") {
    return "mocks";
  }
  return panel;
}

/** Friendly display name for a panel */
export function getPanelDisplayName(panel: Panel): string {
  switch (panel) {
    case "requests":
    case "req-rest":
    case "req-graphql":
    case "req-soap":
    case "req-grpc":
      return "Requests";
    case "mocks":
    case "mock-rest":
    case "mock-graphql":
    case "mock-soap":
    case "mock-grpc":
      return "Mocks";
    case "rules":
      return "Proxy Rules";
    case "sockets":
      return "WebSockets";
    case "webhooks":
      return "Webhooks";
    case "mappings":
      return "Mappings";
    case "services":
      return "Services";
    case "capture":
      return "Capture";
    case "environments":
      return "Environments";
    case "healthbar":
      return "Health Bar";
    case "settings":
      return "Settings";
    case "audit":
      return "Audit Log";
    case "workspace":
      return "Workspace";
    default:
      return panel;
  }
}
