import { AppConfig, Folder, ServiceInfo, SavedRequest, MockRule, ProxyRule, SavedWsConnection, SavedWebhook, Mapping } from "@/types";
import { SearchResultItem } from "./searchTypes";

export function getFolderName(folderId: string | null | undefined, folders: Folder[] = []): string | undefined {
  if (!folderId) return undefined;
  return folders.find((f) => f.id === folderId)?.name;
}

export function matches(query: string, ...fields: (string | undefined | null)[]): boolean {
  if (!query) return true;
  const q = query.toLowerCase().trim();
  return fields.some((f) => f && f.toLowerCase().includes(q));
}
