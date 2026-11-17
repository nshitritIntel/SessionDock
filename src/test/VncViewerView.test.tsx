import { render, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { VncViewerView } from "../components/vnc/VncViewerView";
import { useAppStore, type TerminalTab } from "../stores/appStore";

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), rfb: vi.fn() }));

vi.mock("../api/native", () => ({ nativeInvoke: mocks.invoke }));
vi.mock("zustand/middleware", () => ({ persist: (initializer: unknown) => initializer }));
vi.mock("@novnc/novnc", () => ({
  default: class MockRfb extends EventTarget {
    viewOnly = false;
    focusOnClick = true;
    clipViewport = false;
    dragViewport = false;
    scaleViewport = false;
    resizeSession = false;
    showDotCursor = false;
    qualityLevel = 0;
    compressionLevel = 0;
    constructor(target: Element, url: string, options?: unknown) {
      super();
      mocks.rfb(target, url, options);
    }
    disconnect() {}
    focus() {}
    sendCredentials() {}
  },
}));

const tab: TerminalTab = {
  id: "vnc-tab",
  sessionId: "vnc-session",
  sessionName: "VM console",
  host: "192.0.2.25",
  port: 5901,
  protocol: "vnc",
  status: "connecting",
  pinned: false,
};

describe("VncViewerView", () => {
  beforeEach(() => {
    mocks.invoke.mockReset().mockImplementation((command: string) => {
      if (command === "start_vnc_proxy") {
        return Promise.resolve({ id: "33333333-3333-4333-8333-333333333333", url: "ws://127.0.0.1:49152/vnc/token" });
      }
      return Promise.resolve(undefined);
    });
    mocks.rfb.mockReset();
    useAppStore.setState({ tabs: [tab], activeTabId: tab.id, currentView: "terminal" });
  });

  it("owns the native proxy for the lifetime of the VNC tab", async () => {
    const view = render(<VncViewerView tab={tab} active />);

    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith("start_vnc_proxy", {
      host: "192.0.2.25",
      port: 5901,
    }));
    expect(mocks.rfb).toHaveBeenCalledWith(
      expect.any(HTMLDivElement),
      "ws://127.0.0.1:49152/vnc/token",
      { shared: true, credentials: undefined },
    );

    view.unmount();
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith("stop_vnc_proxy", {
      id: "33333333-3333-4333-8333-333333333333",
    }));
  });

  it("removes a quick-connect password after handing it to noVNC", async () => {
    const passwordTab = { ...tab, password: "temporary-secret" };
    useAppStore.setState({ tabs: [passwordTab] });

    render(<VncViewerView tab={passwordTab} active />);

    await waitFor(() => expect(mocks.rfb).toHaveBeenCalledWith(
      expect.any(HTMLDivElement),
      "ws://127.0.0.1:49152/vnc/token",
      { shared: true, credentials: { username: "", password: "temporary-secret" } },
    ));
    expect(useAppStore.getState().tabs[0].password).toBeUndefined();
  });
});
