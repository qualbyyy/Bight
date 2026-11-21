import type { PermissionState } from "@tauri-apps/plugin-barcode-scanner";

import { isTauri } from "./tauri";

const MOBILE_USER_AGENT = /Android|iPhone|iPad|iPod/i;

export const isMobile =
  isTauri &&
  typeof navigator !== "undefined" &&
  MOBILE_USER_AGENT.test(navigator.userAgent);

export class BarcodePermissionError extends Error {
  readonly permission: PermissionState;

  constructor(permission: PermissionState) {
    super("Доступ к камере не предоставлен.");
    this.name = "BarcodePermissionError";
    this.permission = permission;
  }
}

export async function scanPairingQr(): Promise<string> {
  if (!isMobile) {
    throw new Error("QR-сканер доступен только на мобильных устройствах.");
  }

  const { Format, checkPermissions, requestPermissions, scan } = await import(
    "@tauri-apps/plugin-barcode-scanner"
  );
  let permission = await checkPermissions();
  if (permission !== "granted") {
    permission = await requestPermissions();
  }
  if (permission !== "granted") {
    throw new BarcodePermissionError(permission);
  }

  const result = await scan({
    cameraDirection: "back",
    formats: [Format.QRCode],
  });
  const content = result.content.trim();
  if (!content) {
    throw new Error("QR-код не содержит pairing URI.");
  }
  return content;
}

export async function openCameraSettings(): Promise<void> {
  if (!isMobile) return;
  const { openAppSettings } = await import(
    "@tauri-apps/plugin-barcode-scanner"
  );
  await openAppSettings();
}

export function isScanCancelled(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error);
  return /cancel(?:led|ed)?|dismissed|closed|back/i.test(message);
}
