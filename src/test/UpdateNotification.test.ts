import type { Update } from "@tauri-apps/plugin-updater";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { installUpdate } from "../utils/installUpdate";

const mocks = vi.hoisted(() => ({
  events: [] as string[],
  restartWorker: vi.fn(),
  stopWorker: vi.fn(),
}));

vi.mock("../api/commands", () => ({
  restartMonitorWorkerAfterUpdateFailure: mocks.restartWorker,
  stopMonitorWorkerForUpdate: mocks.stopWorker,
}));

function updateWith(
  install: (options?: { restartAfterInstall?: boolean }) => Promise<void>,
  download: () => Promise<void> = async () => {
    mocks.events.push("download");
  },
): Update {
  return {
    download: vi.fn(download),
    install,
  } as unknown as Update;
}

describe("update worker lifecycle", () => {
  beforeEach(() => {
    mocks.events.length = 0;
    mocks.stopWorker.mockReset().mockImplementation(async () => {
      mocks.events.push("stop");
    });
    mocks.restartWorker.mockReset().mockImplementation(async () => {
      mocks.events.push("restart");
    });
  });

  it("downloads before stopping the worker and installing", async () => {
    const install = vi.fn(async () => {
      mocks.events.push("install");
    });
    const update = updateWith(install);

    await installUpdate(update, vi.fn(), (phase) => mocks.events.push(phase));

    expect(mocks.events).toEqual([
      "downloading",
      "download",
      "stopping-monitoring",
      "stop",
      "installing",
      "install",
    ]);
    expect(install).toHaveBeenCalledWith({ restartAfterInstall: true });
    expect(mocks.restartWorker).not.toHaveBeenCalled();
  });

  it("restarts monitoring when installation fails", async () => {
    const update = updateWith(async () => {
      mocks.events.push("install");
      throw new Error("install failed");
    });

    await expect(installUpdate(update, vi.fn(), vi.fn())).rejects.toThrow("install failed");

    expect(mocks.events).toEqual(["download", "stop", "install", "restart"]);
  });

  it("does not stop monitoring when the verified download fails", async () => {
    const update = updateWith(vi.fn(), async () => {
      throw new Error("download failed");
    });

    await expect(installUpdate(update, vi.fn(), vi.fn())).rejects.toThrow("download failed");

    expect(mocks.stopWorker).not.toHaveBeenCalled();
    expect(mocks.restartWorker).not.toHaveBeenCalled();
    expect(update.install).not.toHaveBeenCalled();
  });

  it("does not install when background monitoring cannot stop", async () => {
    mocks.stopWorker.mockRejectedValueOnce(new Error("worker busy"));
    const update = updateWith(vi.fn());

    await expect(installUpdate(update, vi.fn(), vi.fn())).rejects.toThrow("worker busy");

    expect(mocks.restartWorker).not.toHaveBeenCalled();
    expect(update.install).not.toHaveBeenCalled();
  });

  it("reports when monitoring cannot recover from an install failure", async () => {
    mocks.restartWorker.mockRejectedValueOnce(new Error("restart failed"));
    const update = updateWith(async () => {
      throw new Error("install failed");
    });

    await expect(installUpdate(update, vi.fn(), vi.fn())).rejects.toThrow(
      "Update failed (Error: install failed), and background monitoring could not restart (Error: restart failed)",
    );
  });
});