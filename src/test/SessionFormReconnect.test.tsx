import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SessionForm } from "../components/forms/SessionForm";

const mocks = vi.hoisted(() => ({
  sessions: [] as Array<Record<string, unknown>>,
  credentials: [] as Array<Record<string, unknown>>,
}));

vi.mock("../stores/sessionStore", () => ({
  useSessionStore: (selector: (state: typeof mocks) => unknown) => selector(mocks),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

describe("SessionForm SSH behavior", () => {
  beforeEach(() => {
    mocks.sessions = [];
    mocks.credentials = [];
  });

  it("defaults to disabled and submits an explicit automatic mode", () => {
    const onSubmit = vi.fn();
    render(<SessionForm onSubmit={onSubmit} onCancel={vi.fn()} folders={[]} />);

    const reconnectSelect = screen.getByText("reconnect.mode").parentElement!.querySelector("select")!;
    expect(reconnectSelect).toHaveValue("disabled");
    fireEvent.change(reconnectSelect, { target: { value: "automatic" } });

    const textboxes = screen.getAllByRole("textbox");
    fireEvent.change(textboxes[0], { target: { value: "Core Switch" } });
    fireEvent.change(textboxes[1], { target: { value: "192.0.2.10" } });
    fireEvent.click(screen.getByRole("button", { name: "session.save" }));

    expect(onSubmit).toHaveBeenCalledWith(expect.objectContaining({
      reconnect_mode: "automatic",
    }));
  });

  it("uses the selected credential username until manual credentials are selected", () => {
    mocks.credentials = [{
      id: "credential-1",
      name: "Network operations",
      username: "netadmin",
      authentication_method: "password",
    }];
    render(<SessionForm onSubmit={vi.fn()} onCancel={vi.fn()} folders={[]} />);

    const username = screen.getByPlaceholderText("admin");
    const credentialSelect = screen.getByText("sshAuth.credentialProfile (sshAuth.optional)").parentElement!.querySelector("select")!;
    fireEvent.change(username, { target: { value: "stale-user" } });
    fireEvent.change(credentialSelect, { target: { value: "credential-1" } });

    expect(username).toHaveValue("netadmin");
    expect(username).toBeDisabled();

    fireEvent.change(credentialSelect, { target: { value: "" } });
    expect(username).not.toBeDisabled();
  });
});
