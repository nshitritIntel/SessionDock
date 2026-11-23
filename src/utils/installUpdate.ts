import type { DownloadEvent, Update } from "@tauri-apps/plugin-updater";
import { restartMonitorWorkerAfterUpdateFailure, stopMonitorWorkerForUpdate } from "../api/commands";

export type UpdateInstallPhase = "downloading" | "stopping-monitoring" | "installing";

export async function installUpdate(
  update: Update,
  onDownload: (event: DownloadEvent) => void,
  onPhase: (phase: UpdateInstallPhase) => void,
): Promise<void> {
  onPhase("downloading");
  await update.download(onDownload);
  let monitorWorkerStopped = false;
  try {
    onPhase("stopping-monitoring");
    await stopMonitorWorkerForUpdate();
    monitorWorkerStopped = true;
    onPhase("installing");
    await update.install({ restartAfterInstall: true });
  } catch (error) {
    if (monitorWorkerStopped) {
      try {
        await restartMonitorWorkerAfterUpdateFailure();
      } catch (restartError) {
        throw new Error(
          `Update failed (${String(error)}), and background monitoring could not restart (${String(restartError)})`,
        );
      }
    }
    throw error;
  }
}