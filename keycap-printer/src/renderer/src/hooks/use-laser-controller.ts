import { useCallback, useEffect, useState } from "react";
import { emptyMachineStatus, type EngraveJobData, type JogAxis, type LaserEvent, type MachineStatus, type StreamStatus } from "../../../shared/laser";

type ConnectionState = "disconnected" | "connecting" | "connected";

const EMPTY_STREAM: StreamStatus = { printing: false, current: 0, total: 0, line: "" };

export function useLaserController() {
  const [connection, setConnection] = useState<ConnectionState>("disconnected");
  const [busy, setBusy] = useState(false);
  const [indicatorOn, setIndicatorOn] = useState(false);
  const [machine, setMachine] = useState<MachineStatus>(emptyMachineStatus);
  const [stream, setStream] = useState<StreamStatus>(EMPTY_STREAM);
  const [logs, setLogs] = useState<string[]>([]);

  const appendLog = useCallback((message: string) => {
    const time = new Date().toLocaleTimeString([], { hour12: false });
    setLogs((current) => [...current, `[${time}] ${message}`].slice(-220));
  }, []);

  const run = useCallback(
    async (label: string, action: () => Promise<void>) => {
      try {
        await action();
      } catch (error) {
        appendLog(`${label} failed: ${friendlyIpcError(error)}`);
        throw error;
      }
    },
    [appendLog]
  );

  const connect = useCallback(
    async (baudRate: number) => {
      setConnection("connecting");
      try {
        await run("Connect", () => window.laser.connect(baudRate));
      } catch {
        setConnection("disconnected");
      }
    },
    [run]
  );

  const disconnect = useCallback(async () => {
    await run("Close", () => window.laser.disconnect()).catch(() => undefined);
  }, [run]);

  useEffect(() => {
    const removeListener = window.laser.onEvent((event: LaserEvent) => {
      if (event.type === "log") appendLog(event.message);
      else if (event.type === "connection") setConnection(event.connected ? "connected" : "disconnected");
      else if (event.type === "busy") setBusy(event.busy);
      else if (event.type === "indicator") setIndicatorOn(event.enabled);
      else if (event.type === "machine") setMachine(event.machine);
      else if (event.type === "stream") setStream(event.stream);
    });

    void window.laser.getSnapshot().then((snapshot) => {
      setConnection(snapshot.connected ? "connected" : "disconnected");
      setBusy(snapshot.busy);
      setIndicatorOn(snapshot.indicatorOn);
      setMachine(snapshot.machine);
      setStream(snapshot.stream);
    }).catch((error) => appendLog(`Bridge failed: ${friendlyIpcError(error)}`));

    return removeListener;
  }, [appendLog]);

  return {
    connection,
    busy,
    indicatorOn,
    machine,
    stream,
    logs,
    appendLog,
    connect,
    disconnect,
    probe: () => run("Probe", () => window.laser.probe()).catch(() => undefined),
    unlock: () => run("Unlock", () => window.laser.unlock()).catch(() => undefined),
    home: () => run("Home", () => window.laser.home()).catch(() => undefined),
    jog: (axis: JogAxis, distance: number, feed: number) => run(`Jog ${axis}`, () => window.laser.jog(axis, distance, feed)).catch(() => undefined),
    setIndicator: (enabled: boolean) => run("Indicator", () => window.laser.setIndicator(enabled)).catch(() => undefined),
    runCenteredJob: (job: EngraveJobData, label: string) => run(label, () => window.laser.runCentered(job, label)),
    reset: () => run("Reset", () => window.laser.reset()).catch(() => undefined)
  };
}

function friendlyIpcError(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  return message.replace(/^Error invoking remote method '[^']+': (?:Error: )?/, "");
}
