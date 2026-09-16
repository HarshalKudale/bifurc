import { ipcMain, dialog } from "electron";
import * as fs from "fs";
import * as path from "path";
import { generateCA, getCertStatus } from "@bifurc/engine/proxy/certManager";
import { appDataDir } from "@bifurc/engine/store/appSettings";
import { commandRegistry } from "@/commands/registry";
import { bus } from "@bifurc/engine/eventBus";

// P2 work item 7 — only tls.generate/certStatus/removeCert convert here: all three take no
// params (`z.object({}).strict()` in packages/protocol/src/commands/tls.ts) and are pure
// engine-side today. tls:exportCert/importCert/importKey are SPLIT (see that file's own
// comments — engine should return/accept cert *content*, client owns the save/open dialog) but
// today's handlers still open the dialog themselves; converting them needs the same real
// behavioural split `tls:installCA` already got in work item 5, not a registration-only move,
// so they are left untouched here.
const ctx = { bus };

commandRegistry.register("tls.generate", async () => {
  try {
    const { certPath, keyPath } = await generateCA(appDataDir());
    return { ok: true, certPath, keyPath };
  } catch (e: unknown) {
    return { ok: false, error: e instanceof Error ? e.message : String(e) };
  }
});

commandRegistry.register("tls.certStatus", () => getCertStatus(appDataDir()));

commandRegistry.register("tls.removeCert", () => {
  const certPath = path.join(appDataDir(), "ca-cert.pem");
  const keyPath = path.join(appDataDir(), "ca-key.pem");
  try { if (fs.existsSync(certPath)) fs.unlinkSync(certPath); } catch { /* ignore */ }
  try { if (fs.existsSync(keyPath)) fs.unlinkSync(keyPath); } catch { /* ignore */ }
  return { ok: true };
});

export function registerTlsHandlers() {
  ipcMain.handle("tls:generate", () => commandRegistry.invoke("tls.generate", {}, ctx));

  ipcMain.handle("tls:exportCert", async () => {
    const certPath = path.join(appDataDir(), "ca-cert.pem");
    if (!fs.existsSync(certPath)) return { ok: false, error: "No CA certificate found." };
    const { filePath, canceled } = await dialog.showSaveDialog({
      title: "Export CA Certificate",
      defaultPath: "bifurc-ca.pem",
      filters: [{ name: "Certificate", extensions: ["pem", "crt", "cer"] }],
    });
    if (canceled || !filePath) return { ok: false, canceled: true };
    fs.copyFileSync(certPath, filePath);
    return { ok: true, filePath };
  });

  ipcMain.handle("tls:certStatus", () => commandRegistry.invoke("tls.certStatus", {}, ctx));

  ipcMain.handle("tls:importCert", async () => {
    const { filePaths, canceled } = await dialog.showOpenDialog({
      title: "Select CA Certificate",
      filters: [{ name: "Certificate", extensions: ["pem", "crt", "cer"] }],
      properties: ["openFile"],
    });
    if (canceled || !filePaths[0]) return { ok: false };
    const destPath = path.join(appDataDir(), "ca-cert.pem");
    fs.copyFileSync(filePaths[0], destPath);
    return { ok: true, path: destPath };
  });

  ipcMain.handle("tls:importKey", async () => {
    const { filePaths, canceled } = await dialog.showOpenDialog({
      title: "Select CA Private Key",
      filters: [{ name: "Private Key", extensions: ["pem", "key"] }],
      properties: ["openFile"],
    });
    if (canceled || !filePaths[0]) return { ok: false };
    const destPath = path.join(appDataDir(), "ca-key.pem");
    fs.copyFileSync(filePaths[0], destPath);
    return { ok: true, path: destPath };
  });

  ipcMain.handle("tls:removeCert", () => commandRegistry.invoke("tls.removeCert", {}, ctx));
}

