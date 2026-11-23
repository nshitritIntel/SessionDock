import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { NetworkHealthView } from "../components/views/NetworkHealthView";
import { useSessionStore } from "../stores/sessionStore";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  writeText: vi.fn(),
  incidents: [] as Array<Record<string, unknown>>,
  translate: (key: string, values?: { count?: number; name?: string }) =>
    values?.count !== undefined
      ? `${key}:${values.count}`
      : values?.name
        ? `${key}:${values.name}`
        : key,
}));

vi.mock("../api/native", () => ({ nativeInvoke: mocks.invoke }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({ writeText: mocks.writeText }));
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
  interfaceName: "Ethernet15",
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
  summary: { interfaceCount: 1, downCount: 0, errorCount: 0, logCount: 2 },
};

describe("NetworkHealthView", () => {
  beforeEach(() => {
    useSessionStore.setState({ sessions: [] });
    mocks.writeText.mockReset().mockResolvedValue(undefined);
    mocks.incidents = [];
    mocks.invoke.mockReset().mockImplementation((command: string) => {
      switch (command) {
        case "get_monitor_definitions":
          return Promise.resolve([monitor]);
        case "get_monitor_incidents":
          return Promise.resolve(mocks.incidents);
        case "get_monitor_maintenance":
          return Promise.resolve([]);
        case "get_monitor_samples":
          return Promise.resolve([sample]);
        case "get_monitor_baselines":
          return Promise.resolve([]);
        case "get_monitor_sample_detail":
          return Promise.resolve({
            sample,
            interfaces: [],
            rawOutput: "Relevant recent logs:\nEthernet15 link status changed",
          });
        case "run_monitor_now":
          return Promise.resolve(monitor);
        case "set_monitor_interval":
          return Promise.resolve({ ...monitor, intervalSeconds: 30 });
        case "create_monitor_definition":
          return Promise.resolve({ ...monitor, providerId: "cisco-ios", interfaceName: "GigabitEthernet1/0/1" });
        default:
          return Promise.resolve(undefined);
      }
    });
  });

  it("loads persisted monitor health and runs the selected monitor", async () => {
    render(<NetworkHealthView />);

    expect(await screen.findByText("Leaf interfaces")).toBeInTheDocument();
    expect(screen.getByText(/Ethernet15/)).toBeInTheDocument();
    expect(screen.getAllByText("healthy").length).toBeGreaterThan(0);
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("get_monitor_samples", {
        monitorId: monitor.id,
        limit: 48,
      }),
    );

    fireEvent.click(screen.getByTitle("monitoring.viewSample"));
    expect(await screen.findByText(/Ethernet15 link status changed/)).toBeInTheDocument();
    expect(mocks.invoke).toHaveBeenCalledWith("get_monitor_sample_detail", { sampleId: sample.id });
    fireEvent.click(screen.getByTitle("monitoring.copyOutput"));
    await waitFor(() => expect(mocks.writeText).toHaveBeenCalledWith(
      "Relevant recent logs:\nEthernet15 link status changed",
    ));

    fireEvent.change(screen.getByLabelText(/monitoring\.intervalFor/), { target: { value: "30" } });
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith("set_monitor_interval", {
      id: monitor.id,
      intervalSeconds: 30,
    }));

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

    fireEvent.click(screen.getByText("monitoring.tabs.baselines"));
    expect(screen.getByText("monitoring.baselineHelp")).toBeInTheDocument();
  });

  it("refreshes samples automatically without Run Now", async () => {
    vi.useFakeTimers();
    try {
      render(<NetworkHealthView />);
      await act(async () => {
        await Promise.resolve();
        await Promise.resolve();
      });
      const before = mocks.invoke.mock.calls.filter(([command]) => command === "get_monitor_samples").length;

      await act(async () => {
        await vi.advanceTimersByTimeAsync(5_000);
      });

      const after = mocks.invoke.mock.calls.filter(([command]) => command === "get_monitor_samples").length;
      expect(after).toBeGreaterThan(before);
      expect(mocks.invoke).not.toHaveBeenCalledWith("run_monitor_now", expect.anything());
    } finally {
      vi.useRealTimers();
    }
  });

  it("reopens an acknowledged incident", async () => {
    const incidentId = "33333333-3333-4333-8333-333333333333";
    mocks.incidents = [{
      id: incidentId,
      monitorId: monitor.id,
      fingerprint: "cisco-ios:interface-errors",
      severity: "warning",
      state: "acknowledged",
      title: "Interface errors detected",
      details: "GigabitEthernet1/0/1 reports nonzero error or discard counters",
      startedAt: "2026-04-01T10:00:00Z",
      lastSeenAt: "2026-04-01T10:01:00Z",
      occurrenceCount: 2,
    }];
    render(<NetworkHealthView />);

    await screen.findByText("Leaf interfaces");
    fireEvent.click(screen.getByText("monitoring.tabs.incidents"));
    expect(await screen.findByText("Interface errors detected")).toBeInTheDocument();
    fireEvent.click(await screen.findByText("monitoring.unacknowledge"));

    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith(
      "unacknowledge_monitor_incident",
      { id: incidentId },
    ));
  });

  it("creates a Cisco IOS monitor from an explicitly supported platform", async () => {
    useSessionStore.setState({ sessions: [{
      id: monitor.sessionId, name: "Router 1", host: "192.0.2.20", port: 22, protocol: "ssh",
      username: "admin", authentication_method: "ssh_agent", jump_host_ids: [], favorite: false,
      device_type: "Cisco IOS", vendor: "Cisco", connection_timeout: 30, keepalive_interval: 60,
      created_at: "2026-01-01", updated_at: "2026-01-01", connection_count: 0,
      bmc_use_https: true, bmc_viewer_mode: "web", bmc_ignore_tls_errors: false,
      bmc_open_console_automatically: false, bmc_open_fullscreen: false,
      bmc_timeout_seconds: 30, bmc_redfish_enabled: true, bmc_cookie_persistence: "application",
    }] });
    render(<NetworkHealthView />);
    await screen.findByText("Leaf interfaces");
    fireEvent.click(screen.getByText("monitoring.addMonitor"));

    expect(screen.getByLabelText("monitoring.platform")).toHaveValue("cisco-ios");
    fireEvent.change(screen.getByLabelText("monitoring.interface"), {
      target: { value: "GigabitEthernet1/0/1" },
    });
    fireEvent.click(screen.getByText("common.create"));

    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith("create_monitor_definition", {
      request: expect.objectContaining({
        sessionId: monitor.sessionId,
        providerId: "cisco-ios",
        monitorKind: "interfaces",
        interfaceName: "GigabitEthernet1/0/1",
      }),
    }));
  });
});
