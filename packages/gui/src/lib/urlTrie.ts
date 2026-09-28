// Shared URL trie for building hierarchical tree views
import { TreeItem, TreeItemIndex } from "react-complex-tree";

export interface TrieNode<T> {
  name: string;
  children: Map<string, TrieNode<T>>;
  items: T[];
}

export function urlSegments(url: string): string[] {
  try {
    const u = new URL(url);
    const parts = u.pathname.replace(/\/$/, "").split("/").filter(Boolean);
    return [u.host, ...parts];
  } catch {
    return [url];
  }
}

export function compressNode<T>(node: TrieNode<T>): TrieNode<T> {
  const newChildren = new Map<string, TrieNode<T>>();
  for (const [, child] of node.children) {
    const c = compressNode(child);
    newChildren.set(c.name, c);
  }
  if (node.items.length === 0 && newChildren.size === 1) {
    const child = [...newChildren.values()][0];
    return { name: `${node.name}/${child.name}`, children: child.children, items: child.items };
  }
  return { name: node.name, children: newChildren, items: node.items };
}

export interface TrieItemData<T> {
  name: string;
  item?: T;
  fullPath?: string;
}

export function buildTrieItems<T extends { id: string }>(
  entries: T[],
  getUrl: (entry: T) => string,
  leafPrefix: string,
  getLeafName: (entry: T) => string,
): {
  items: Record<TreeItemIndex, TreeItem<TrieItemData<T>>>;
  branchIds: TreeItemIndex[];
} {
  const trieRoot = new Map<string, TrieNode<T>>();
  for (const entry of entries) {
    const segs = urlSegments(getUrl(entry));
    let cur = trieRoot;
    for (let i = 0; i < segs.length; i++) {
      const seg = segs[i];
      if (!cur.has(seg)) cur.set(seg, { name: seg, children: new Map(), items: [] });
      const node = cur.get(seg)!;
      if (i === segs.length - 1) node.items.push(entry);
      else cur = node.children;
    }
  }

  const compressed = new Map<string, TrieNode<T>>();
  for (const [, node] of trieRoot) {
    const c = compressNode(node);
    compressed.set(c.name, c);
  }

  const items: Record<TreeItemIndex, TreeItem<TrieItemData<T>>> = {};
  const branchIds: TreeItemIndex[] = [];

  function processNode(node: TrieNode<T>, path: string): TreeItemIndex {
    const nodeId = `seg-${path}`;
    const children: TreeItemIndex[] = [];

    for (const entry of node.items) {
      const leafId = `${leafPrefix}-${entry.id}`;
      items[leafId] = {
        index: leafId,
        isFolder: false,
        data: { name: getLeafName(entry), item: entry, fullPath: getUrl(entry) },
      };
      children.push(leafId);
    }

    for (const [, child] of node.children) {
      children.push(processNode(child, `${path}/${child.name}`));
    }

    items[nodeId] = {
      index: nodeId,
      isFolder: true,
      children,
      data: { name: node.name, fullPath: path },
    };
    branchIds.push(nodeId);
    return nodeId;
  }

  const rootChildren: TreeItemIndex[] = [];
  for (const [, node] of compressed) {
    rootChildren.push(processNode(node, node.name));
  }

  items["root"] = { index: "root", isFolder: true, children: rootChildren, data: { name: "root" } };
  branchIds.push("root");
  return { items, branchIds };
}

export function collectLeafIds<T>(
  itemId: TreeItemIndex,
  items: Record<TreeItemIndex, TreeItem<TrieItemData<T>>>,
): string[] {
  const item = items[itemId];
  if (!item) return [];
  if (item.data.item) return [(item.data.item as unknown as { id: string }).id];
  return (item.children ?? []).flatMap((c) => collectLeafIds(c, items));
}
