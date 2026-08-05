import { existsSync } from "node:fs";
import { ReadlineParser, SerialPort } from "serialport";
import {
  buildHyLaserJogCommand,
  emptyMachineStatus,
  HY_LASER_INDICATOR_FEED,
  HY_LASER_INDICATOR_POWER,
  parseHyLaserStatus,
  type EngraveJobData,
  type JogAxis,
  type LaserEvent,
  type LaserSnapshot,
  type MachineStatus,
  type StreamStatus
} from "../shared/laser";
import { buildLightBurnGCode } from "./lightburn-gcode";

const HY_LASER_VENDOR_ID = 0x303a;
const HY_LASER_PRODUCT_ID = 0x4001;
const INDICATOR_HEARTBEAT_MS = 1_000;

interface PendingCommand {
  command: string;
  lines: string[];
  showBusy: boolean;
  resolve: (lines: string[]) => void;
  reject: (error: Error) => void;
  timer: NodeJS.Timeout;
}

interface PendingStatus {
  resolve: (line: string) => void;
  reject: (error: Error) => void;
  timer: NodeJS.Timeout;
}

export class NativeLaserController {
  private port: SerialPort | null = null;
  private parser: ReadlineParser | null = null;
  private pendingCommands: PendingCommand[] = [];
  private pendingStatuses: PendingStatus[] = [];
  private statusRequest: Promise<string> | null = null;
  private statusTimer: NodeJS.Timeout | null = null;
  private indicatorHeartbeatTimer: NodeJS.Timeout | null = null;
  private connected = false;
  private busy = false;
  private indicatorOn = false;
  private indicatorFocusMode = false;
  private printing = false;
  private cancelled = false;
  private closing = false;
  private machine: MachineStatus = emptyMachineStatus();
  private streamStatus: StreamStatus = { printing: false, current: 0, total: 0, line: "" };

  constructor(private readonly emit: (event: LaserEvent) => void) {}

  getSnapshot(): LaserSnapshot {
    return {
      connected: this.connected,
      busy: this.busy,
      indicatorOn: this.indicatorOn,
      machine: this.machine,
      stream: this.streamStatus
    };
  }

  async connect(baudRate = 115200): Promise<void> {
    if (this.connected || this.port) await this.disconnect();
    const path = await findHyLaserPath();
    this.log(`Opening ${path} at ${baudRate}.`);

    const port = new SerialPort({
      path,
      baudRate,
      autoOpen: false,
      lock: true,
      dataBits: 8,
      stopBits: 1,
      parity: "none",
      rtscts: false,
      xon: false,
      xoff: false,
      xany: false
    });
    this.port = port;

    try {
      await new Promise<void>((resolve, reject) => {
        port.open((error) => (error ? reject(error) : resolve()));
      });

      this.attachTextParser();
      port.on("error", (error) => this.log(`Serial error: ${error.message}`));
      port.on("close", (error) => this.handleClose(error));

      this.connected = true;
      this.cancelled = false;
      this.setIndicatorState(false);
      this.emit({ type: "connection", connected: true });
      this.log(`Connected to HY-Laser on ${path}.`);
      await this.writeRaw("\x18");
      await sleep(700);
      await this.writeRaw("\r\n");
      await this.sendCommand("$32=1", 5000);
      this.indicatorFocusMode = false;
      await this.probe();
      this.startStatusPolling();
    } catch (error) {
      await this.disconnect();
      throw new Error(`Failed to open ${path}: ${friendlyError(error)}`);
    }
  }

  async disconnect(): Promise<void> {
    this.stopStatusPolling();
    this.clearIndicatorHeartbeat();
    if (this.port?.isOpen) {
      try {
        if (this.connected && this.indicatorFocusMode) await this.stopIndicator();
        await this.writeRaw("\x18");
        await sleep(500);
        await this.writeRaw("M5\nM9\n$32=1\n");
        await sleep(100);
      } catch (error) {
        this.log(`Laser shutdown failed: ${friendlyError(error)}`);
      }
    }
    this.indicatorFocusMode = false;
    this.setIndicatorState(false);
    this.connected = false;
    this.cancelled = true;
    this.printing = false;
    this.rejectPending(new Error("Disconnected"));

    const port = this.port;
    this.port = null;
    this.parser?.removeAllListeners();
    this.parser = null;

    if (port?.isOpen) {
      this.closing = true;
      try {
        await new Promise<void>((resolve, reject) => {
          port.close((error) => (error ? reject(error) : resolve()));
        });
      } catch (error) {
        this.log(`Close failed: ${friendlyError(error)}`);
      } finally {
        this.closing = false;
      }
    }

    this.emit({ type: "connection", connected: false });
    this.machine = emptyMachineStatus();
    this.emit({ type: "machine", machine: this.machine });
    this.setBusy(false);
    this.emitStream(0, 0, "");
  }

  async probe(): Promise<void> {
    this.ensureConnected();
    await this.sendCommand("$I", 4000);
    await this.sendCommand("$G", 4000);
    await this.queryStatus();
  }

  async unlock(): Promise<void> {
    await this.sendCommand("$X");
  }

  async home(): Promise<void> {
    if (this.indicatorFocusMode) await this.stopIndicator();
    await this.sendCommand("$H", 60000);
  }

  async jog(axis: JogAxis, distance: number, feed: number): Promise<void> {
    this.ensureConnected();
    if (this.printing) throw new Error("Cannot jog while a job is streaming");
    if (this.machine.state !== "Idle") throw new Error(`Cannot jog while the machine is ${this.machine.state}`);

    const command = buildHyLaserJogCommand(axis, distance, feed);
    this.log(`Jog ${axis} ${distance > 0 ? "+" : ""}${distance.toFixed(3)} mm at ${Math.round(feed)} mm/min.`);
    await this.sendCommand(command, 5000);
    await this.waitForIdle(30_000);
  }

  async setIndicator(enabled: boolean): Promise<void> {
    this.ensureConnected();
    if (this.printing) throw new Error("Cannot change the indicator while a job is streaming");

    if (!enabled) {
      if (this.indicatorOn || this.indicatorFocusMode) await this.stopIndicator();
      return;
    }

    if (this.machine.state !== "Idle") throw new Error(`Cannot enable the indicator while the machine is ${this.machine.state}`);
    if (!this.indicatorOn) {
      await this.sendCommand("M5", 5000);
      await this.sendCommand("$32=0", 5000);
      this.indicatorFocusMode = true;
      try {
        await this.sendCommand("M3", 5000);
        await this.sendCommand(`G1 F${HY_LASER_INDICATOR_FEED} S${HY_LASER_INDICATOR_POWER}`, 5000);
        this.setIndicatorState(true);
        this.startIndicatorHeartbeat();
        this.log(`Laser indicator on at S${HY_LASER_INDICATOR_POWER} (2%) in stationary G1 mode.`);
      } catch (error) {
        await this.stopIndicator().catch((shutdownError) => {
          this.log(`Indicator recovery failed: ${friendlyError(shutdownError)}`);
        });
        throw error;
      }
    }
  }

  async runCentered(job: EngraveJobData, label: string): Promise<void> {
    this.ensureConnected();
    if (this.printing) throw new Error("A job is already streaming");
    if (this.machine.state !== "Idle") throw new Error(`Cannot start while the machine is ${this.machine.state}`);
    this.stopStatusPolling();
    const commands = buildLightBurnGCode(job);
    let completed = false;
    try {
      if (this.indicatorFocusMode) await this.stopIndicator(false);
      else await this.sendCommand("M5", 5000);
      await this.sendCommand("$32=0", 5000);
      await this.queryStatus();
      if (this.machine.cover === "open") throw new Error("Close the cover before engraving");
      if (this.machine.state !== "Idle") throw new Error(`Cannot start while the machine is ${this.machine.state}`);

      const center = parseMachinePoint(this.machine.position);
      this.log(`Current machine position X${center.x.toFixed(3)} Y${center.y.toFixed(3)} is the icon center.`);
      this.log(`${label}: LightBurn-compatible GRBL-M3, ${job.powerPercent}% power, ${job.speedPercent}% speed.`);

      this.printing = true;
      this.cancelled = false;
      this.emitStream(0, commands.length, "Streaming GRBL-M3");
      for (const [index, command] of commands.entries()) {
        if (this.cancelled) throw new Error("Engraving cancelled");
        await this.sendCommand(command, 15_000, false);
        this.emitStream(index + 1, commands.length, command);
      }
      this.emitStream(commands.length, commands.length, "Finishing motion");
      await this.waitForIdle(10 * 60_000, 500, "Timed out waiting for engraving to complete");
      completed = true;
    } finally {
      this.printing = false;
      this.emitStream(this.streamStatus.current, commands.length, "");

      if (this.connected && this.port?.isOpen) {
        if (this.cancelled) await sleep(700);
        await this.sendCommand("M5", 5000).catch((error) => this.log(`Post-job M5 failed: ${friendlyError(error)}`));
        await this.sendCommand("M9", 5000).catch((error) => this.log(`Post-job M9 failed: ${friendlyError(error)}`));
        await this.sendCommand("$32=1", 5000).catch((error) => this.log(`Post-job laser-mode restore failed: ${friendlyError(error)}`));
        await this.queryStatus().catch(() => undefined);
        this.startStatusPolling();
      }
    }

    if (completed) this.log(`${label} complete.`);
  }

  private attachTextParser(): void {
    const port = this.port;
    if (!port?.isOpen || this.parser) return;
    this.parser = port.pipe(new ReadlineParser({ delimiter: "\n" }));
    this.parser.on("data", (line: string) => this.handleLine(line.trim()));
  }

  async reset(): Promise<void> {
    this.ensureConnected();
    const interruptedJob = this.printing;
    this.cancelled = true;
    await this.sendRealtime("\x18");
    this.setIndicatorState(false);
    this.rejectPending(new Error("Controller reset"));
    this.indicatorFocusMode = false;
    if (interruptedJob) {
      this.log("Emergency GRBL reset sent during engraving.");
      return;
    }
    await sleep(700);
    await this.writeRaw("\r\n");
    await this.sendCommand("$X", 5000);
    await this.sendCommand("M5", 5000);
    await this.sendCommand("M9", 5000);
    await this.sendCommand("$32=1", 5000);
    await this.queryStatus();
  }

  private async stopIndicator(restoreLaserMode = true): Promise<void> {
    const wasOn = this.indicatorOn;
    await this.sendCommand("M5", 5000);
    this.setIndicatorState(false);
    await this.sendCommand("G0", 5000);
    if (this.indicatorFocusMode && restoreLaserMode) {
      await this.sendCommand("$32=1", 5000);
    }
    this.indicatorFocusMode = false;
    if (wasOn) this.log(restoreLaserMode ? "Laser indicator off; laser mode restored." : "Laser indicator off.");
  }

  private async sendCommand(command: string, timeout = 8000, showBusy = true): Promise<string[]> {
    this.ensureConnected();
    const line = command.trim();
    if (!line) return [];

    let pending!: PendingCommand;
    const response = new Promise<string[]>((resolve, reject) => {
      pending = {
        command: line,
        lines: [],
        showBusy,
        resolve,
        reject,
        timer: setTimeout(() => {
          this.pendingCommands = this.pendingCommands.filter((item) => item !== pending);
          this.syncCommandBusyState();
          reject(new Error(`Timeout waiting for ${line}`));
        }, timeout)
      };
      this.pendingCommands.push(pending);
      this.syncCommandBusyState();
    });
    void response.catch(() => undefined);

    try {
      await this.writeRaw(`${line}\n`);
    } catch (error) {
      this.rejectCommand(pending, error instanceof Error ? error : new Error(String(error)));
    }
    return response;
  }

  private async queryStatus(): Promise<string> {
    this.ensureConnected();
    if (this.statusRequest) return this.statusRequest;

    let pending!: PendingStatus;
    const response = new Promise<string>((resolve, reject) => {
      pending = {
        resolve,
        reject,
        timer: setTimeout(() => {
          this.pendingStatuses = this.pendingStatuses.filter((item) => item !== pending);
          reject(new Error("Status timeout"));
        }, 2500)
      };
      this.pendingStatuses.push(pending);
    });
    let tracked!: Promise<string>;
    tracked = response.finally(() => {
      if (this.statusRequest === tracked) this.statusRequest = null;
    });
    void tracked.catch(() => undefined);
    this.statusRequest = tracked;

    try {
      await this.writeRaw("?");
    } catch (error) {
      this.pendingStatuses = this.pendingStatuses.filter((item) => item !== pending);
      clearTimeout(pending.timer);
      pending.reject(error instanceof Error ? error : new Error(String(error)));
    }
    return tracked;
  }

  private async sendRealtime(command: string): Promise<void> {
    await this.writeRaw(command);
    this.log(`>> ${command === "\x18" ? "ctrl-x" : command}`);
  }

  private writeRaw(text: string): Promise<void> {
    const port = this.port;
    if (!port?.isOpen) return Promise.reject(new Error("Serial port is not open"));
    return new Promise((resolve, reject) => {
      port.write(text, (error) => (error ? reject(error) : resolve()));
    });
  }

  private handleLine(rawLine: string): void {
    const line = rawLine.trim();
    if (!line) return;
    this.log(`<< ${line}`);

    if (line.startsWith("<") && line.endsWith(">")) {
      const previousCover = this.machine.cover;
      this.machine = parseHyLaserStatus(line) ?? emptyMachineStatus();
      this.emit({ type: "machine", machine: this.machine });
      if (this.machine.state.startsWith("Alarm")) this.setIndicatorState(false);
      if (this.machine.cover !== previousCover) {
        if (this.machine.cover === "open") this.log("Cover open (standard GRBL door interlock active).");
        else if (this.machine.cover === "closed") this.log("Cover closed (standard GRBL door interlock clear).");
        else if (previousCover === "unknown") this.log("Cover state is not reported by firmware; relying on the device's hardware interlock.");
      }
      if (this.machine.cover === "open" && this.indicatorOn) {
        void this.setIndicator(false).catch((error) => this.log(`Door indicator shutdown failed: ${friendlyError(error)}`));
      }
      const pending = this.pendingStatuses.shift();
      if (pending) {
        clearTimeout(pending.timer);
        pending.resolve(line);
      }
      return;
    }

    const pending = this.pendingCommands[0];
    if (!pending) return;
    if (line === "ok" || line.startsWith("error:") || line.startsWith("ALARM:")) {
      if (line.startsWith("ALARM:")) this.setIndicatorState(false);
      this.pendingCommands.shift();
      clearTimeout(pending.timer);
      this.syncCommandBusyState();
      if (line === "ok") pending.resolve(pending.lines);
      else pending.reject(new Error(`${line} after ${pending.command}`));
      return;
    }
    pending.lines.push(line);
  }

  private handleClose(error: Error | null): void {
    if (this.closing) return;
    this.connected = false;
    this.cancelled = true;
    this.printing = false;
    this.setIndicatorState(false);
    this.stopStatusPolling();
    this.rejectPending(error ?? new Error("Serial connection closed"));
    this.port = null;
    this.parser?.removeAllListeners();
    this.parser = null;
    this.emit({ type: "connection", connected: false });
    this.machine = emptyMachineStatus();
    this.emit({ type: "machine", machine: this.machine });
    this.emitStream(this.streamStatus.current, this.streamStatus.total, "");
    this.log(error ? `Serial connection closed: ${error.message}` : "Serial connection closed.");
  }

  private rejectCommand(pending: PendingCommand, error: Error): void {
    this.pendingCommands = this.pendingCommands.filter((item) => item !== pending);
    clearTimeout(pending.timer);
    pending.reject(error);
    this.syncCommandBusyState();
  }

  private rejectPending(error: Error): void {
    for (const pending of this.pendingCommands) {
      clearTimeout(pending.timer);
      pending.reject(error);
    }
    for (const pending of this.pendingStatuses) {
      clearTimeout(pending.timer);
      pending.reject(error);
    }
    this.pendingCommands = [];
    this.pendingStatuses = [];
    this.setBusy(false);
  }

  private startStatusPolling(): void {
    this.stopStatusPolling();
    this.statusTimer = setInterval(() => {
      if (this.connected && this.pendingStatuses.length === 0) {
        void this.queryStatus().catch(() => undefined);
      }
    }, 500);
  }

  private stopStatusPolling(): void {
    if (this.statusTimer) clearInterval(this.statusTimer);
    this.statusTimer = null;
  }

  private ensureConnected(): void {
    if (!this.connected || !this.port?.isOpen) throw new Error("No serial connection");
  }

  private setIndicatorState(enabled: boolean): void {
    if (!enabled) {
      this.clearIndicatorHeartbeat();
    }
    if (this.indicatorOn === enabled) return;
    this.indicatorOn = enabled;
    this.emit({ type: "indicator", enabled });
  }

  private startIndicatorHeartbeat(): void {
    this.clearIndicatorHeartbeat();
    this.indicatorHeartbeatTimer = setInterval(() => {
      if (
        !this.connected ||
        !this.indicatorOn ||
        this.printing ||
        this.busy ||
        this.machine.state !== "Idle" ||
        this.pendingCommands.length > 0
      ) return;

      void this.sendCommand(
        `G1 F${HY_LASER_INDICATOR_FEED} S${HY_LASER_INDICATOR_POWER}`,
        5000,
        false
      ).catch((error) => {
        this.log(`Indicator heartbeat failed: ${friendlyError(error)}`);
        void this.stopIndicator().catch((shutdownError) => {
          this.log(`Indicator shutdown failed: ${friendlyError(shutdownError)}`);
        });
      });
    }, INDICATOR_HEARTBEAT_MS);
  }

  private clearIndicatorHeartbeat(): void {
    if (this.indicatorHeartbeatTimer) clearInterval(this.indicatorHeartbeatTimer);
    this.indicatorHeartbeatTimer = null;
  }

  private async waitForIdle(
    timeout: number,
    minimumWait = 150,
    timeoutMessage = "Timed out waiting for jog to complete"
  ): Promise<void> {
    const started = Date.now();
    this.setBusy(true);
    try {
      while (Date.now() - started < timeout) {
        await this.queryStatus();
        if (this.machine.state === "Idle" && Date.now() - started >= minimumWait) return;
        if (this.machine.state.startsWith("Alarm")) throw new Error(`Machine entered ${this.machine.state}`);
        await sleep(100);
      }
      throw new Error(timeoutMessage);
    } finally {
      this.setBusy(false);
    }
  }

  private setBusy(busy: boolean): void {
    if (this.busy === busy) return;
    this.busy = busy;
    this.emit({ type: "busy", busy });
  }

  private syncCommandBusyState(): void {
    this.setBusy(this.pendingCommands.some((command) => command.showBusy));
  }

  private emitStream(current: number, total: number, line: string): void {
    this.streamStatus = { printing: this.printing, current, total, line };
    this.emit({ type: "stream", stream: this.streamStatus });
  }

  private log(message: string): void {
    this.emit({ type: "log", message });
  }
}

type NativePortInfo = Awaited<ReturnType<typeof SerialPort.list>>[number];

export async function findHyLaserPath(ports?: NativePortInfo[]): Promise<string> {
  const availablePorts = ports ?? await SerialPort.list();
  const device = availablePorts.find(
    (port) => idMatches(port.vendorId, HY_LASER_VENDOR_ID) && idMatches(port.productId, HY_LASER_PRODUCT_ID)
  );
  if (!device) throw new Error("HY-Laser Device (VID 303A / PID 4001) was not found");

  if (process.platform === "darwin" && device.path.startsWith("/dev/tty.")) {
    const calloutPath = device.path.replace("/dev/tty.", "/dev/cu.");
    if (existsSync(calloutPath)) return calloutPath;
  }
  return device.path;
}

function idMatches(value: string | undefined, expected: number): boolean {
  if (!value) return false;
  const text = value.trim().toLowerCase().replace(/^0x/, "");
  return /^[0-9a-f]+$/.test(text) && Number.parseInt(text, 16) === expected;
}

function friendlyError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function parseMachinePoint(position: string): { x: number; y: number } {
  const [x, y] = position.split(",").map((value) => Number(value.trim()));
  if (!Number.isFinite(x) || !Number.isFinite(y)) throw new Error("Machine did not report a usable X/Y position");
  return { x, y };
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
