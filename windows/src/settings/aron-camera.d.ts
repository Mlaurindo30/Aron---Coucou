export interface CameraDevice {
  kind: string;
  deviceId: string;
  label: string;
}

export interface SavedCamera {
  camera_device: string;
  camera_label: string;
}

export interface CameraOption {
  deviceId: string;
  label: string;
  active: boolean;
}

export function cameraOptions(
  devices: readonly CameraDevice[],
  saved: SavedCamera,
): CameraOption[];

export function cameraState(
  devices: readonly CameraDevice[],
  permissionDenied: boolean,
): { state: "ready" | "empty" | "denied"; message: string };
