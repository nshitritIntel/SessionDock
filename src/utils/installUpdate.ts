import { relaunch } from "@tauri-apps/plugin-process";
import type { DownloadEvent, Update } from "@tauri-apps/plugin-updater";
import { restartMonitorWorkerAfterUpdateFailure, stopMonitorWorkerForUpdate } from "../api/commands";

export async function installUpdate(
  update: Update,
  onDownload: (event: DownloadEvent) => void,
): Promise<void> {
  await update.download(onDownload);
  let monitorWorkerStopped = false;
  try {
    await stopMonitorWorkerForUpdate();
    monitorWorkerStopped = true;
    await update.install();
    await relaunch();
  } catch (error) {
    if (monitorWorkerStopped) {
      await restartMonitorWorkerAfterUpdateFailure().catch(() => undefined);
    }
    throw error;
  }
}