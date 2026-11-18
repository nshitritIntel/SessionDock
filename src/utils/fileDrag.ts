export const LOCAL_FILE_DRAG_TYPE = "application/sessiondock-local-file";
export const REMOTE_FILE_DRAG_TYPE = "application/sessiondock-remote-file";

export interface FileDragPayload {
  name: string;
  path: string;
  kind?: "file" | "directory";
  items?: FileDragItem[];
}

export interface FileDragItem {
  name: string;
  path: string;
  kind: "file" | "directory";
}

const MAX_DRAG_PAYLOAD_LENGTH = 1_048_576;
const MAX_DRAG_ITEMS = 256;
const INTERNAL_DRAG_SENTINEL = "SessionDock internal file transfer";
let activeFileDrag: { type: string; payload: FileDragPayload } | undefined;

function copyPayload(payload: FileDragPayload): FileDragPayload {
  return {
    name: payload.name,
    path: payload.path,
    ...(payload.kind ? { kind: payload.kind } : {}),
    ...(payload.items ? { items: payload.items.map((item) => ({ ...item })) } : {}),
  };
}

function isValidFileDragPayload(payload: Partial<FileDragPayload>): payload is FileDragPayload {
  const validPrimary = typeof payload.name === "string"
    && payload.name.length > 0
    && payload.name.length <= 255
    && typeof payload.path === "string"
    && payload.path.length > 0
    && payload.path.length <= 32_768
    && !payload.path.includes("\0")
    && (payload.kind === undefined || payload.kind === "file" || payload.kind === "directory");
  if (!validPrimary) return false;
  if (payload.items === undefined) return true;
  return payload.items.length > 0
    && payload.items.length <= MAX_DRAG_ITEMS
    && payload.items.every((item) => isValidFileDragItem(item));
}

function isValidFileDragItem(item: Partial<FileDragItem>): item is FileDragItem {
  return typeof item.name === "string"
    && item.name.length > 0
    && item.name.length <= 255
    && typeof item.path === "string"
    && item.path.length > 0
    && item.path.length <= 32_768
    && !item.path.includes("\0")
    && (item.kind === "file" || item.kind === "directory");
}

export function hasFileDragType(dataTransfer: DataTransfer, type: string): boolean {
  return Array.from(dataTransfer.types ?? []).includes(type) || activeFileDrag?.type === type;
}

export function setFileDragPayload(dataTransfer: DataTransfer, type: string, payload: FileDragPayload): void {
  const serialized = JSON.stringify(payload);
  if (!isValidFileDragPayload(payload) || serialized.length > MAX_DRAG_PAYLOAD_LENGTH) {
    activeFileDrag = undefined;
    return;
  }
  activeFileDrag = { type, payload: copyPayload(payload) };
  dataTransfer.effectAllowed = "copy";
  dataTransfer.setData("text/plain", INTERNAL_DRAG_SENTINEL);
  try {
    dataTransfer.setData(type, serialized);
  } catch {
    return;
  }
}

export function clearFileDragPayload(): void {
  activeFileDrag = undefined;
}

export function readFileDragPayload(dataTransfer: DataTransfer, type: string): FileDragPayload | undefined {
  let raw = "";
  try {
    raw = dataTransfer.getData(type);
  } catch {
    raw = "";
  }
  if (!raw) return activeFileDrag?.type === type ? copyPayload(activeFileDrag.payload) : undefined;
  if (raw.length > MAX_DRAG_PAYLOAD_LENGTH) return undefined;
  try {
    const payload = JSON.parse(raw) as Partial<FileDragPayload>;
    if (!isValidFileDragPayload(payload)) return undefined;
    return copyPayload(payload);
  } catch {
    return activeFileDrag?.type === type ? copyPayload(activeFileDrag.payload) : undefined;
  }
}