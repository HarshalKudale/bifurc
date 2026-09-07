import React, { createContext, useContext } from 'react';
import { TreeContextType } from './FolderTree.types';

export const FolderTreeContext = createContext<TreeContextType | null>(null);

export function useFolderTreeContext() {
  const context = useContext(FolderTreeContext);
  if (!context) {
    throw new Error('useFolderTreeContext must be used within a FolderTreeProvider');
  }
  return context;
}
