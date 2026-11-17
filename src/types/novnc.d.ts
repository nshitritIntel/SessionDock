declare module "@novnc/novnc" {
  export interface RfbCredentials {
    username?: string;
    password?: string;
    target?: string;
  }

  export default class RFB extends EventTarget {
    constructor(target: Element, url: string, options?: { shared?: boolean; credentials?: RfbCredentials });
    viewOnly: boolean;
    focusOnClick: boolean;
    clipViewport: boolean;
    dragViewport: boolean;
    scaleViewport: boolean;
    resizeSession: boolean;
    showDotCursor: boolean;
    qualityLevel: number;
    compressionLevel: number;
    disconnect(): void;
    focus(): void;
    sendCredentials(credentials: RfbCredentials): void;
    addEventListener(type: "connect", listener: (event: CustomEvent<Record<string, never>>) => void): void;
    addEventListener(type: "disconnect", listener: (event: CustomEvent<{ clean: boolean }>) => void): void;
    addEventListener(type: "credentialsrequired", listener: (event: CustomEvent<{ types: string[] }>) => void): void;
    addEventListener(type: "securityfailure", listener: (event: CustomEvent<{ status: number; reason?: string }>) => void): void;
  }
}
