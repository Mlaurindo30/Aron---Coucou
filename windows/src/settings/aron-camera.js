/**
 * @typedef {{ kind: string, deviceId: string, label: string }} CameraDevice
 * @typedef {{ camera_device: string, camera_label: string }} SavedCamera
 * @typedef {{ deviceId: string, label: string, active: boolean }} CameraOption
 */

/**
 * @param {readonly CameraDevice[]} devices
 * @param {SavedCamera} saved
 * @returns {CameraOption[]}
 */
export function cameraOptions(devices, saved) {
  const cameras = devices.filter((device) => device.kind === "videoinput");
  const selected =
    cameras.find((device) => device.deviceId === saved.camera_device && device.deviceId) ??
    cameras.find((device) => saved.camera_label && device.label === saved.camera_label);

  return cameras.map((device) => ({
    deviceId: device.deviceId,
    label: device.label || "Camera",
    active: device === selected,
  }));
}

/**
 * @param {readonly CameraDevice[]} devices
 * @param {boolean} permissionDenied
 * @returns {{ state: "ready" | "empty" | "denied", message: string }}
 */
export function cameraState(devices, permissionDenied) {
  if (permissionDenied) {
    return {
      state: "denied",
      message: "Camera permission was denied. Allow access and try again.",
    };
  }
  const count = devices.filter((device) => device.kind === "videoinput").length;
  if (count === 0) return { state: "empty", message: "No cameras found." };
  return {
    state: "ready",
    message: `Found ${count} camera${count === 1 ? "" : "s"}.`,
  };
}
