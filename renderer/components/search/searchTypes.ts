import { Panel } from "@/lib/panelRegistry";

export interface SearchResultItem {
  id: string;
  title: string;
  subtitle?: string;
  method?: string;
  panel: Panel;
  isOpenTab: boolean;
  tabId?: string;
  isDraft?: boolean;
  folderName?: string;
  section: string;
  entityType: "tab" | "request" | "mock" | "rule" | "websocket" | "webhook" | "mapping" | "service" | "environment";
  original?: any;
  isMapped?: boolean;
  mappingLabel?: string;
}

export interface SearchSection {
  title: string;
  items: SearchResultItem[];
}

export interface SearchResult {
  sections: SearchSection[];
  flatItems: SearchResultItem[];
}
