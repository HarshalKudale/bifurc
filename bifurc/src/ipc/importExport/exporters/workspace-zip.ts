import * as fs from "fs";
import * as path from "path";
import archiver from "archiver";
import { wsDir } from "@/store/workspaceFs";
import type { ExportResult } from "@/ipc/importExport/types";

export async function run(wsId: string, filePath: string): Promise<ExportResult> {
  return new Promise((resolve) => {
    const sourceDir = wsDir(wsId);
    if (!fs.existsSync(sourceDir)) {
      resolve({ ok: false, error: "Workspace directory not found" });
      return;
    }

    const output = fs.createWriteStream(filePath);
    const archive = archiver("zip", { zlib: { level: 6 } });

    output.on("close", () => resolve({ ok: true, filePath }));
    archive.on("error", (err) => resolve({ ok: false, error: err.message }));

    archive.pipe(output);

    // Add all workspace files, skipping .git
    archive.glob("**/*", {
      cwd: sourceDir,
      dot: true,
      ignore: [".git/**", ".git"],
    });

    archive.finalize();
  });
}
