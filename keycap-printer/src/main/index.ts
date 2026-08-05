import { app, BrowserWindow, ipcMain, shell, type IpcMainInvokeEvent } from "electron";
import { join } from "node:path";
import { NativeLaserController } from "./laser-controller";
import { isEngraveJobData, LASER_CHANNELS, type LaserEvent } from "../shared/laser";

const trustedWebContents = new Set<number>();
const laser = new NativeLaserController(broadcastLaserEvent);
let shutdownPromise: Promise<void> | null = null;
let quitReady = false;

function shutdownLaser(): Promise<void> {
  if (!shutdownPromise) {
    shutdownPromise = laser.disconnect().finally(() => {
      shutdownPromise = null;
    });
  }
  return shutdownPromise;
}

function broadcastLaserEvent(event: LaserEvent): void {
  for (const window of BrowserWindow.getAllWindows()) {
    if (!window.isDestroyed() && trustedWebContents.has(window.webContents.id)) {
      window.webContents.send(LASER_CHANNELS.event, event);
    }
  }
}

function requireTrustedSender(event: IpcMainInvokeEvent): void {
  if (!trustedWebContents.has(event.sender.id)) throw new Error("Untrusted laser IPC request");
}

function registerLaserIpc(): void {
  ipcMain.handle(LASER_CHANNELS.snapshot, (event) => {
    requireTrustedSender(event);
    return laser.getSnapshot();
  });
  ipcMain.handle(LASER_CHANNELS.connect, async (event, baudRate: unknown) => {
    requireTrustedSender(event);
    if (!Number.isInteger(baudRate) || Number(baudRate) < 300 || Number(baudRate) > 921600) {
      throw new Error("Invalid baud rate");
    }
    await laser.connect(Number(baudRate));
  });
  ipcMain.handle(LASER_CHANNELS.disconnect, async (event) => {
    requireTrustedSender(event);
    await laser.disconnect();
  });
  ipcMain.handle(LASER_CHANNELS.probe, async (event) => {
    requireTrustedSender(event);
    await laser.probe();
  });
  ipcMain.handle(LASER_CHANNELS.unlock, async (event) => {
    requireTrustedSender(event);
    await laser.unlock();
  });
  ipcMain.handle(LASER_CHANNELS.home, async (event) => {
    requireTrustedSender(event);
    await laser.home();
  });
  ipcMain.handle(LASER_CHANNELS.jog, async (event, axis: unknown, distance: unknown, feed: unknown) => {
    requireTrustedSender(event);
    if (axis !== "X" && axis !== "Y") throw new Error("Invalid jog axis");
    if (typeof distance !== "number" || !Number.isFinite(distance) || distance === 0 || Math.abs(distance) > 100) {
      throw new Error("Invalid jog distance");
    }
    if (typeof feed !== "number" || !Number.isFinite(feed) || feed < 1 || feed > 10_000) {
      throw new Error("Invalid jog feed");
    }
    await laser.jog(axis, distance, feed);
  });
  ipcMain.handle(LASER_CHANNELS.indicator, async (event, enabled: unknown) => {
    requireTrustedSender(event);
    if (typeof enabled !== "boolean") throw new Error("Invalid indicator state");
    await laser.setIndicator(enabled);
  });
  ipcMain.handle(LASER_CHANNELS.runCentered, async (event, job: unknown, label: unknown) => {
    requireTrustedSender(event);
    if (!isEngraveJobData(job)) throw new Error("Invalid engraving vector payload");
    if (typeof label !== "string" || label.length > 40) throw new Error("Invalid job label");
    await laser.runCentered(job, label);
  });
  ipcMain.handle(LASER_CHANNELS.reset, async (event) => {
    requireTrustedSender(event);
    await laser.reset();
  });
}

function createWindow(): void {
  const window = new BrowserWindow({
    width: 1320,
    height: 860,
    minWidth: 1040,
    minHeight: 720,
    show: false,
    title: "Keycap Printer",
    backgroundColor: "#f4f6f8",
    webPreferences: {
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      preload: join(__dirname, "../preload/index.cjs")
    }
  });

  const contentsId = window.webContents.id;
  trustedWebContents.add(contentsId);
  window.on("closed", () => {
    trustedWebContents.delete(contentsId);
    void shutdownLaser();
  });
  window.once("ready-to-show", () => window.show());

  window.webContents.setWindowOpenHandler(({ url }) => {
    if (/^https?:\/\//i.test(url)) void shell.openExternal(url);
    return { action: "deny" };
  });

  const devUrl = process.env.ELECTRON_RENDERER_URL;
  const allowedOrigin = devUrl ? new URL(devUrl).origin : "file://";
  window.webContents.on("will-navigate", (event, url) => {
    const origin = url.startsWith("file://") ? "file://" : new URL(url).origin;
    if (origin !== allowedOrigin) event.preventDefault();
  });

  window.webContents.on("did-fail-load", (_event, code, description, url) => {
    console.error(`Renderer failed to load ${url}: ${code} ${description}`);
  });

  if (devUrl) void window.loadURL(devUrl);
  else void window.loadFile(join(__dirname, "../renderer/index.html"));
}

app.whenReady().then(() => {
  registerLaserIpc();
  createWindow();
  app.on("activate", () => {
    if (BrowserWindow.getAllWindows().length === 0) createWindow();
  });
});

app.on("window-all-closed", () => {
  if (process.platform !== "darwin") app.quit();
});

app.on("before-quit", (event) => {
  if (quitReady) return;
  event.preventDefault();
  void shutdownLaser().finally(() => {
    quitReady = true;
    app.quit();
  });
});

for (const signal of ["SIGINT", "SIGTERM"] as const) {
  process.once(signal, () => {
    void shutdownLaser().finally(() => process.exit(0));
  });
}
