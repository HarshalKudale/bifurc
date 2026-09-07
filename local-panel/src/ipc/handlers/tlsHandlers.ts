import { ipcMain, dialog } from "electron";
import * as fs from "fs";
import * as path from "path";
import { generateCA, installCA, getCertStatus } from "@/proxy/certManager";
import { appDataDir } from "@/store/appSettings";

export function registerTlsHandlers() {
  ipcMain.handle("tls:generate", async () => {
    try {
      const { certPath, keyPath } = await generateCA(appDataDir());
      return { ok: true, certPath, keyPath };
    } catch (e: unknown) {
      return { ok: false, error: e instanceof Error ? e.message : String(e) };
    }
  });

  ipcMain.handle("tls:installCA", () => {
    const certPath = path.join(appDataDir(), "ca-cert.pem");
    if (!fs.existsSync(certPath)) return { ok: false, error: "No CA certificate found. Generate one first." };
    return installCA(certPath);
  });

  ipcMain.handle("tls:exportCert", async () => {
    const certPath = path.join(appDataDir(), "ca-cert.pem");
    if (!fs.existsSync(certPath)) return { ok: false, error: "No CA certificate found." };
    const { filePath, canceled } = await dialog.showSaveDialog({
      title: "Export CA Certificate",
      defaultPath: "local-panel-ca.pem",
      filters: [{ name: "Certificate", extensions: ["pem", "crt", "cer"] }],
    });
    if (canceled || !filePath) return { ok: false, canceled: true };
    fs.copyFileSync(certPath, filePath);
    return { ok: true, filePath };
  });

  ipcMain.handle("tls:certStatus", () => getCertStatus(appDataDir()));

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

  ipcMain.handle("tls:removeCert", () => {
    const certPath = path.join(appDataDir(), "ca-cert.pem");
    const keyPath = path.join(appDataDir(), "ca-key.pem");
    try { if (fs.existsSync(certPath)) fs.unlinkSync(certPath); } catch { /* ignore */ }
    try { if (fs.existsSync(keyPath)) fs.unlinkSync(keyPath); } catch { /* ignore */ }
    return { ok: true };
  });
}
