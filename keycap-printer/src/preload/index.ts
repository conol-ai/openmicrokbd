import { contextBridge, ipcRenderer } from "electron";
import { LASER_CHANNELS, type LaserApi, type LaserEvent } from "../shared/laser";

const laserApi: LaserApi = {
  getSnapshot: () => ipcRenderer.invoke(LASER_CHANNELS.snapshot),
  connect: (baudRate) => ipcRenderer.invoke(LASER_CHANNELS.connect, baudRate),
  disconnect: () => ipcRenderer.invoke(LASER_CHANNELS.disconnect),
  probe: () => ipcRenderer.invoke(LASER_CHANNELS.probe),
  unlock: () => ipcRenderer.invoke(LASER_CHANNELS.unlock),
  home: () => ipcRenderer.invoke(LASER_CHANNELS.home),
  jog: (axis, distance, feed) => ipcRenderer.invoke(LASER_CHANNELS.jog, axis, distance, feed),
  setIndicator: (enabled) => ipcRenderer.invoke(LASER_CHANNELS.indicator, enabled),
  runCentered: (job, label) => ipcRenderer.invoke(LASER_CHANNELS.runCentered, job, label),
  reset: () => ipcRenderer.invoke(LASER_CHANNELS.reset),
  onEvent: (listener) => {
    const handler = (_event: Electron.IpcRendererEvent, event: LaserEvent) => listener(event);
    ipcRenderer.on(LASER_CHANNELS.event, handler);
    return () => ipcRenderer.removeListener(LASER_CHANNELS.event, handler);
  }
};

contextBridge.exposeInMainWorld("laser", laserApi);
