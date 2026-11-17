import type { Update } from "@tauri-apps/plugin-updater";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { installUpdate } from "../utils/installUpdate";

const mocks = vi.hoisted(() => ({
  events: [] as string[],
  restartWorker: vi.fn(),
  stopWorker: vi.fn(),
  relaunch: vi.fn(),
}));

vi.mock("../api/commands", () => ({
  restartMonitorWorkerAfterUpdateFailure: mocks.restartWorker,
  stopMonitorWorkerForUpdate: mocks.stopWorker,
}));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: mocks.relaunch }));

function updateWith(install: () => Promise<void>): Update {
  return {
    download: vi.fn(async () => {
      mocks.events.push("download");
    }),
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
    mocks.relaunch.mockReset().mockImplementation(async () => {
      mocks.events.push("relaunch");
    });
  });

  it("downloads before stopping the worker and installing", async () => {
    const update = updateWith(async () => {
      mocks.events.push("install");
    });

    await installUpdate(update, vi.fn());

    expect(mocks.events).toEqual(["download", "stop", "install", "relaunch"]);
    expect(mocks.restartWorker).not.toHaveBeenCalled();
  });

  it("restarts monitoring when installation fails", async () => {
    const update = updateWith(async () => {
      mocks.events.push("install");
      throw new Error("install failed");
    });

    await expect(installUpdate(update, vi.fn())).rejects.toThrow("install failed");

    expect(mocks.events).toEqual(["download", "stop", "install", "restart"]);
  });
});