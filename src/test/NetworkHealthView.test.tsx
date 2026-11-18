import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { NetworkHealthView } from "../components/views/NetworkHealthView";
import { useSessionStore } from "../stores/sessionStore";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  translate: (key: string, values?: { count?: number; name?: string }) =>
    values?.count !== undefined
      ? `${key}:${values.count}`
      : values?.name
        ? `${key}:${values.name}`
        : key,
}));

vi.mock("../api/native", () => ({ nativeInvoke: mocks.invoke }));
vi.mock("zustand/middleware", () => ({
  persist: (initializer: unknown) => initializer,
}));
vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: mocks.translate }),
}));

const monitor = {
  id: "11111111-1111-4111-8111-111111111111",
  sessionId: "22222222-2222-4222-8222-222222222222",
  sessionName: "Leaf 1",
  host: "192.0.2.10",
  providerId: "arista-eos",
  monitorKind: "interfaces",
  displayName: "Leaf interfaces",
  intervalSeconds: 300,
  retentionHours: 168,
  enabled: true,
  latestStatus: "healthy",
  latestObservedAt: "2026-04-01T10:00:00Z",
  openIncidentCount: 0,
  createdAt: "2026-04-01T09:00:00Z",
  updatedAt: "2026-04-01T09:00:00Z",
};

const sample = {
  id: 1,
  monitorId: monitor.id,
  observedAt: "2026-04-01T10:00:00Z",
  status: "healthy",
  durationMs: 120,
  summary: { interfaceCount: 48, downCount: 0, errorCount: 0 },
};

describe("NetworkHealthView", () => {
  beforeEach(() => {
    useSessionStore.setState({ sessions: [] });
    mocks.invoke.mockReset().mockImplementation((command: string) => {
      switch (command) {
        case "get_monitor_definitions":
          return Promise.resolve([monitor]);
        case "get_monitor_incidents":
          return Promise.resolve([]);
        case "get_monitor_maintenance":
          return Promise.resolve([]);
        case "get_monitor_samples":
          return Promise.resolve([sample]);
        case "get_monitor_baselines":
          return Promise.resolve([]);
        case "run_monitor_now":
          return Promise.resolve(monitor);
        default:
          return Promise.resolve(undefined);
      }
    });
  });

  it("loads persisted monitor health and runs the selected monitor", async () => {
    render(<NetworkHealthView />);

    expect(await screen.findByText("Leaf interfaces")).toBeInTheDocument();
    expect(screen.getAllByText("healthy").length).toBeGreaterThan(0);
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("get_monitor_samples", {
        monitorId: monitor.id,
        limit: 48,
      }),
    );

    fireEvent.click(screen.getByTitle("monitoring.runNow"));

    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("run_monitor_now", {
        id: monitor.id,
      }),
    );
    await waitFor(() =>
      expect(
        mocks.invoke.mock.calls.filter(
          ([command]) => command === "get_monitor_definitions",
        ),
      ).toHaveLength(2),
    );
  });
});
