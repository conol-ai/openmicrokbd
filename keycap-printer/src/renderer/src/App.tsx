import { icons } from "lucide";
import type { IconNode } from "lucide";
import * as simpleIcons from "simple-icons";
import type { SimpleIcon } from "simple-icons";
import {
  Activity,
  ArrowDown,
  ArrowLeft,
  ArrowRight,
  ArrowUp,
  Cable,
  Crosshair,
  Eye,
  EyeOff,
  Gauge,
  LockOpen,
  Play,
  RotateCcw,
  Route,
  Search,
  ShieldAlert,
  ShieldCheck,
  ShieldQuestion,
  SlidersHorizontal,
  Unplug,
  Usb
} from "lucide-react";
import { useEffect, useMemo, useRef, useState, type ReactNode, type UIEvent } from "react";
import { Checkbox } from "@/components/ui/checkbox";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Progress } from "@/components/ui/progress";
import { Slider } from "@/components/ui/slider";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { IconGlyph } from "@/components/icon-glyph";
import { ToolpathPreview } from "@/components/toolpath-preview";
import { useLaserController } from "@/hooks/use-laser-controller";
import {
  buildIconJob,
  buildSimpleIconJob,
  DEFAULT_SETTINGS,
  ENGRAVE_POWER_PERCENT,
  ENGRAVE_SPEED_PERCENT,
  feedFromPercent,
  feedToPercent,
  normalizeSettings,
  powerFromPercent,
  powerToPercent,
  type LaserSettings
} from "@/lib/toolpath";
import { cn } from "@/lib/utils";

const ICON_BATCH_SIZE = 180;
const JOG_FEED = 1000;
const JOG_STEPS = [0.1, 1, 10] as const;
const COMMON_ICONS = [
  "Keyboard", "Command", "Code", "Terminal", "Cpu", "Zap", "Power", "Settings", "Wifi", "Bluetooth",
  "Volume2", "Mic", "Moon", "Sun", "ChevronUp", "ChevronDown", "ChevronLeft", "ChevronRight", "ArrowUp",
  "ArrowDown", "ArrowLeft", "ArrowRight", "Play", "Pause", "Circle", "Square", "Triangle", "Star", "Heart",
  "Home", "Mail", "Search", "Delete"
];
const COMMON_BRANDS = [
  "apple", "google", "microsoft", "github", "youtube", "spotify", "discord", "instagram", "facebook", "x",
  "whatsapp", "tiktok", "twitch", "steam", "netflix", "amazon", "reddit", "slack", "notion", "figma",
  "visualstudiocode", "docker", "android", "react", "typescript"
];

type IconLibrary = "lucide" | "simple";

interface IconEntryBase {
  key: string;
  label: string;
  search: string;
}

interface LucideIconEntry extends IconEntryBase {
  kind: "lucide";
  node: IconNode;
}

interface SimpleIconEntry extends IconEntryBase {
  kind: "simple";
  path: string;
}

type IconEntry = LucideIconEntry | SimpleIconEntry;

const LUCIDE_ICON_ENTRIES: LucideIconEntry[] = Object.entries(icons)
  .filter((entry): entry is [string, IconNode] => Array.isArray(entry[1]))
  .map(([key, node]): LucideIconEntry => ({ kind: "lucide", key, node, label: iconLabel(key), search: `${key} ${iconLabel(key)}`.toLowerCase() }))
  .sort((a, b) => {
    const first = COMMON_ICONS.indexOf(a.key);
    const second = COMMON_ICONS.indexOf(b.key);
    if (first !== -1 || second !== -1) return (first === -1 ? 999 : first) - (second === -1 ? 999 : second);
    return a.label.localeCompare(b.label);
  });

const SIMPLE_ICON_ENTRIES: SimpleIconEntry[] = Object.values(simpleIcons)
  .filter(isSimpleIcon)
  .map((icon): SimpleIconEntry => ({ kind: "simple", key: icon.slug, path: icon.path, label: icon.title, search: `${icon.title} ${icon.slug}`.toLowerCase() }))
  .sort((a, b) => {
    const first = COMMON_BRANDS.indexOf(a.key);
    const second = COMMON_BRANDS.indexOf(b.key);
    if (first !== -1 || second !== -1) return (first === -1 ? 999 : first) - (second === -1 ? 999 : second);
    return a.label.localeCompare(b.label);
  });

export function App() {
  const [settings, setSettings] = useState<LaserSettings>(DEFAULT_SETTINGS);
  const [iconLibrary, setIconLibrary] = useState<IconLibrary>("lucide");
  const [selectedKeys, setSelectedKeys] = useState<Record<IconLibrary, string>>({ lucide: "Keyboard", simple: "github" });
  const [query, setQuery] = useState("");
  const [visibleIcons, setVisibleIcons] = useState(ICON_BATCH_SIZE);
  const [jogStep, setJogStep] = useState<number>(1);
  const consoleRef = useRef<HTMLPreElement>(null);
  const controller = useLaserController();

  const iconEntries: IconEntry[] = iconLibrary === "lucide" ? LUCIDE_ICON_ENTRIES : SIMPLE_ICON_ENTRIES;
  const selectedKey = selectedKeys[iconLibrary];
  const selected = iconEntries.find((entry) => entry.key === selectedKey) ?? iconEntries[0];
  const filteredIcons = useMemo(
    () => iconEntries.filter((entry) => !query.trim() || entry.search.includes(query.trim().toLowerCase())),
    [iconEntries, query]
  );
  const jobResult = useMemo(() => {
    try {
      if (!selected) return { job: null, error: null };
      const job = selected.kind === "lucide"
        ? buildIconJob(selected.node, settings)
        : buildSimpleIconJob(selected.path, settings);
      return { job, error: null };
    } catch (error) {
      return { job: null, error: error instanceof Error ? error.message : String(error) };
    }
  }, [selected, settings]);
  const job = jobResult.job;

  const connected = controller.connection === "connected";
  const connecting = controller.connection === "connecting";
  const operating = controller.busy || controller.stream.printing;
  const coverOpen = controller.machine.cover === "open";
  const coverClosed = controller.machine.cover === "closed";
  const canJog = connected && !operating && controller.machine.state === "Idle";
  const progress = controller.stream.total ? (controller.stream.current / controller.stream.total) * 100 : 0;

  useEffect(() => {
    consoleRef.current?.scrollTo({ top: consoleRef.current.scrollHeight });
  }, [controller.logs]);

  useEffect(() => {
    if (jobResult.error) controller.appendLog(`Icon generation failed: ${jobResult.error}`);
  }, [jobResult.error, controller.appendLog]);

  function updateSetting<Key extends keyof LaserSettings>(key: Key, value: LaserSettings[Key]) {
    setSettings((current) => normalizeSettings({ ...current, [key]: value }));
  }

  function applyEngravePreset() {
    setSettings((current) => normalizeSettings({
      ...current,
      power: powerFromPercent(ENGRAVE_POWER_PERCENT),
      engraveFeed: feedFromPercent(ENGRAVE_SPEED_PERCENT)
    }));
  }

  function handleIconScroll(event: UIEvent<HTMLDivElement>) {
    const target = event.currentTarget;
    if (target.scrollTop + target.clientHeight >= target.scrollHeight - 180 && visibleIcons < filteredIcons.length) {
      setVisibleIcons((current) => current + ICON_BATCH_SIZE);
    }
  }

  function changeIconLibrary(value: string) {
    if (value !== "lucide" && value !== "simple") return;
    setIconLibrary(value);
    setQuery("");
    setVisibleIcons(ICON_BATCH_SIZE);
  }

  function selectIcon(key: string) {
    setSelectedKeys((current) => ({ ...current, [iconLibrary]: key }));
  }

  async function startJob() {
    if (!job || !job.stats.fitsKeycap) return;
    try {
      await controller.runCenteredJob({
        segments: job.segments,
        intensities: job.intensities,
        powerPercent: powerToPercent(settings.power),
        speedPercent: feedToPercent(settings.engraveFeed),
        passes: settings.passes
      }, "Engrave");
    } catch {
      // The controller records the actionable error in the console.
    }
  }

  const rasterSummary = job ? [
    "Protocol: LightBurn-compatible GRBL-M3 raster",
    `Library: ${selected.kind === "lucide" ? "Lucide" : "Simple Icons"}`,
    `Icon: ${selected.label}`,
    `Pixels: ${job.stats.pixelCount}`,
    `Scanlines: ${job.stats.scanlineCount}`,
    `Runs: ${job.stats.motionSegments}`,
    `Grayscale levels: ${job.stats.grayscaleLevels}`,
    "Resolution: 0.1 mm",
    selected.kind === "lucide" ? `Line width: ${settings.lineWidth.toFixed(1)} mm` : "Fill: solid brand silhouette",
    `Power: ${powerToPercent(settings.power).toFixed(0)}%`,
    `Speed: ${feedToPercent(settings.engraveFeed).toFixed(0)}%`,
    `Passes: ${settings.passes}`,
    "Origin: current machine position"
  ].join("\n") : "";

  return (
    <main className="grid h-screen min-h-[720px] grid-cols-[276px_minmax(0,1fr)] overflow-hidden bg-background text-foreground">
      <aside className="flex min-h-0 flex-col border-r border-border bg-sidebar">
        <div className="flex h-[73px] shrink-0 items-center gap-3 border-b border-border px-5">
          <div className="grid size-9 place-items-center rounded-md border border-foreground/20 bg-accent text-accent-foreground">
            <Crosshair className="size-5" strokeWidth={2.2} />
          </div>
          <div className="min-w-0">
            <h1 className="truncate text-[15px] font-semibold">Keycap Printer</h1>
            <p className="mt-0.5 text-[11px] text-muted-foreground">HY-Laser / GRBL-M3</p>
          </div>
        </div>

        <div className="space-y-2 p-3 pb-2">
          <Tabs value={iconLibrary} onValueChange={changeIconLibrary}>
            <TabsList className="grid w-full grid-cols-2">
              <TabsTrigger value="lucide">Lucide</TabsTrigger>
              <TabsTrigger value="simple">Simple Icons</TabsTrigger>
            </TabsList>
          </Tabs>
          <div className="relative">
            <Search className="pointer-events-none absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
            <Input
              value={query}
              onChange={(event) => { setQuery(event.target.value); setVisibleIcons(ICON_BATCH_SIZE); }}
              className="pl-8"
              placeholder={iconLibrary === "lucide" ? "Search Lucide icons" : "Search brand logos"}
              aria-label={iconLibrary === "lucide" ? "Search Lucide icons" : "Search Simple Icons logos"}
            />
          </div>
        </div>

        <div className="flex items-center justify-between px-4 py-1.5 text-[11px] font-medium text-muted-foreground">
          <span>Icons</span>
          <span className="font-mono">{filteredIcons.length}</span>
        </div>
        <div className="icon-scroll grid min-h-0 flex-1 auto-rows-[68px] grid-cols-3 gap-1.5 overflow-y-auto px-3 pb-3" onScroll={handleIconScroll}>
          {filteredIcons.slice(0, visibleIcons).map((entry) => (
            <button
              key={entry.key}
              type="button"
              title={entry.label}
              aria-label={entry.label}
              aria-pressed={entry.key === selectedKey}
              onClick={() => selectIcon(entry.key)}
              className={cn(
                "grid min-w-0 grid-rows-[28px_18px] place-items-center gap-1 rounded-md border border-transparent px-1 py-2 text-muted-foreground outline-none transition-colors hover:border-border hover:bg-muted/70 hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/30",
                entry.key === selectedKey && "border-primary/30 bg-primary/10 text-primary"
              )}
            >
              <IconGlyph
                node={entry.kind === "lucide" ? entry.node : undefined}
                brandPath={entry.kind === "simple" ? entry.path : undefined}
                className="size-5"
              />
              <span className="w-full truncate text-center text-[10px] leading-none">{entry.label}</span>
            </button>
          ))}
        </div>
      </aside>

      <section className="min-w-0 overflow-y-auto">
        <header className="sticky top-0 z-20 flex h-[73px] items-center justify-between gap-5 border-b border-border bg-background/95 px-5 backdrop-blur">
          <div className="min-w-0">
            <p className="truncate text-[11px] text-muted-foreground">USB CDC / VID 303A / PID 4001 / 115200 baud</p>
            <h2 className="mt-0.5 truncate text-xl font-semibold tracking-normal">{selected.label}</h2>
          </div>
          <div className={cn(
            "flex h-8 shrink-0 items-center gap-2 rounded-full border px-3 text-xs font-medium",
            connected ? "border-primary/25 bg-primary/10 text-primary" : "border-border bg-muted/50 text-muted-foreground",
            controller.stream.printing && "border-destructive/25 bg-destructive/10 text-destructive",
            controller.indicatorOn && "border-warning/40 bg-warning/10 text-warning-foreground"
          )}>
            <span className={cn("size-2 rounded-full bg-muted-foreground", connected && "bg-primary", controller.stream.printing && "bg-destructive", controller.indicatorOn && "bg-warning")} />
            {controller.stream.printing ? "Engraving" : controller.indicatorOn ? "Indicator 2%" : connecting ? "Connecting" : connected ? "Connected" : "Disconnected"}
          </div>
        </header>

        <div className="grid grid-cols-1 gap-4 p-4 min-[1180px]:grid-cols-[minmax(0,1fr)_340px]">
          <section className="flex min-h-[500px] flex-col overflow-hidden rounded-lg border border-border bg-card">
            <PanelTitle icon={<Crosshair />} title="Toolpath" trailing={job?.stats.fitsKeycap ? "Within boundary" : "Check boundary"} />
            <ToolpathPreview job={job} settings={settings} />
            <div className="grid grid-cols-4 divide-x divide-border border-t border-border bg-card">
              <Metric label="Pixels" value={job?.stats.pixelCount ?? 0} />
              <Metric label="Scan rows" value={job?.stats.scanlineCount ?? 0} />
              <Metric label="Gray levels" value={job?.stats.grayscaleLevels ?? 0} />
              <Metric label="Cut path" value={`${job?.stats.cutsMm.toFixed(1) ?? "0.0"} mm`} />
            </div>
          </section>

          <section className="overflow-hidden rounded-lg border border-border bg-card min-[1180px]:row-span-2">
            <PanelTitle icon={<SlidersHorizontal />} title="Job settings" />
            <fieldset disabled={controller.stream.printing} className="space-y-5 p-4 disabled:opacity-60">
              <SettingsGroup title="Layout">
                <div className="grid grid-cols-2 gap-2.5">
                  <NumberField label="Keycap W" value={settings.keycapWidth} min={1} max={80} step={0.1} onChange={(value) => updateSetting("keycapWidth", value)} />
                  <NumberField label="Keycap H" value={settings.keycapHeight} min={1} max={80} step={0.1} onChange={(value) => updateSetting("keycapHeight", value)} />
                </div>
                <RangeField label="Icon size" value={settings.iconSize} min={1} max={20} step={0.1} suffix="mm" onChange={(value) => updateSetting("iconSize", value)} />
                {selected.kind === "lucide" ? (
                  <RangeField label="Line width" value={settings.lineWidth} min={0.1} max={1.5} step={0.1} suffix="mm" onChange={(value) => updateSetting("lineWidth", value)} />
                ) : null}
                <div className="grid grid-cols-2 gap-2.5">
                  <NumberField label="Fine X" value={settings.offsetX} min={-500} max={500} step={0.1} onChange={(value) => updateSetting("offsetX", value)} />
                  <NumberField label="Fine Y" value={settings.offsetY} min={-500} max={500} step={0.1} onChange={(value) => updateSetting("offsetY", value)} />
                </div>
                <RangeField label="Rotation" value={settings.rotation} min={-180} max={180} step={1} suffix="deg" onChange={(value) => updateSetting("rotation", value)} />
                <div className="grid grid-cols-2 gap-2">
                  <CheckField label="Mirror X" checked={settings.mirrorX} onChange={(value) => updateSetting("mirrorX", value)} />
                  <CheckField label="Mirror Y" checked={settings.mirrorY} onChange={(value) => updateSetting("mirrorY", value)} />
                </div>
              </SettingsGroup>

              <SettingsGroup title="Laser">
                <Button size="sm" className="w-full" onClick={applyEngravePreset}>
                  <Gauge />Engrave 100/10
                </Button>
                <div className="grid grid-cols-3 gap-2.5">
                  <NumberField label="Power %" value={powerToPercent(settings.power)} min={0} max={100} step={1} onChange={(value) => updateSetting("power", powerFromPercent(value))} />
                  <NumberField label="Speed %" value={feedToPercent(settings.engraveFeed)} min={1} max={100} step={1} onChange={(value) => updateSetting("engraveFeed", feedFromPercent(value))} />
                  <NumberField label="Passes" value={settings.passes} min={1} max={20} step={1} onChange={(value) => updateSetting("passes", value)} />
                </div>
              </SettingsGroup>
            </fieldset>
          </section>

          <section className="rounded-lg border border-border bg-card">
            <PanelTitle icon={<Usb />} title="Device" />
            <div className="space-y-3 p-4">
              <div className="grid grid-cols-3 gap-2">
                <Button variant="default" disabled={connected || connecting} onClick={() => void controller.connect(settings.baudRate)}><Cable />Connect</Button>
                <Button disabled={!connected || operating} onClick={() => void controller.probe()}><Activity />Probe</Button>
                <Button disabled={!connected || operating} onClick={() => void controller.unlock()}><LockOpen />Unlock</Button>
              </div>
              <div className="overflow-hidden rounded-md border border-border bg-muted/30">
                <div className="grid grid-cols-3 divide-x divide-border">
                  <Metric label="State" value={controller.machine.state} compact />
                  <Metric
                    label="Cover"
                    value={
                      <span
                        className={cn("flex items-center gap-1.5", coverOpen ? "text-destructive" : coverClosed ? "text-primary" : "text-muted-foreground")}
                        title={controller.machine.cover === "unavailable" ? "The firmware does not report its hardware cover interlock" : undefined}
                      >
                        {coverOpen ? <ShieldAlert className="size-3.5" /> : coverClosed ? <ShieldCheck className="size-3.5" /> : <ShieldQuestion className="size-3.5" />}
                        {coverOpen ? "Open" : coverClosed ? "Closed" : controller.machine.cover === "unavailable" ? "Hardware" : "Unknown"}
                      </span>
                    }
                    compact
                  />
                  <Metric label="Inputs" value={controller.machine.pins || "None"} compact mono />
                </div>
                <div className="border-t border-border">
                  <Metric label="Position" value={controller.machine.position} compact mono />
                </div>
              </div>
              <div className="grid grid-cols-[104px_minmax(0,1fr)] items-center gap-3 rounded-md border border-border bg-muted/20 p-3">
                <div className="grid size-[104px] grid-cols-3 grid-rows-3 gap-1" aria-label="Manual X and Y movement">
                  <Button
                    size="icon"
                    className="col-start-2 row-start-1 size-8 self-end justify-self-center"
                    disabled={!canJog}
                    aria-label="Jog Y positive"
                    title={`Y+ ${jogStep} mm`}
                    onClick={() => void controller.jog("Y", jogStep, JOG_FEED)}
                  >
                    <ArrowUp />
                  </Button>
                  <Button
                    size="icon"
                    className="col-start-1 row-start-2 size-8 self-center justify-self-end"
                    disabled={!canJog}
                    aria-label="Jog X negative"
                    title={`X- ${jogStep} mm`}
                    onClick={() => void controller.jog("X", -jogStep, JOG_FEED)}
                  >
                    <ArrowLeft />
                  </Button>
                  <Button
                    size="icon"
                    variant="default"
                    className="col-start-2 row-start-2 size-8 self-center justify-self-center"
                    disabled={!connected || operating}
                    aria-label="Home machine"
                    title="Home machine"
                    onClick={() => void controller.home()}
                  >
                    <Crosshair />
                  </Button>
                  <Button
                    size="icon"
                    className="col-start-3 row-start-2 size-8 self-center justify-self-start"
                    disabled={!canJog}
                    aria-label="Jog X positive"
                    title={`X+ ${jogStep} mm`}
                    onClick={() => void controller.jog("X", jogStep, JOG_FEED)}
                  >
                    <ArrowRight />
                  </Button>
                  <Button
                    size="icon"
                    className="col-start-2 row-start-3 size-8 self-start justify-self-center"
                    disabled={!canJog}
                    aria-label="Jog Y negative"
                    title={`Y- ${jogStep} mm`}
                    onClick={() => void controller.jog("Y", -jogStep, JOG_FEED)}
                  >
                    <ArrowDown />
                  </Button>
                </div>
                <div className="min-w-0 space-y-2">
                  <div className="text-[11px] font-medium text-muted-foreground">Jog step</div>
                  <div className="grid grid-cols-3 overflow-hidden rounded-md border border-border" role="group" aria-label="Jog step in millimeters">
                    {JOG_STEPS.map((step, index) => (
                      <button
                        key={step}
                        type="button"
                        onClick={() => setJogStep(step)}
                        aria-pressed={jogStep === step}
                        className={cn(
                          "h-8 text-[11px] font-medium outline-none transition-colors hover:bg-muted focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring/30",
                          index > 0 && "border-l border-border",
                          jogStep === step ? "bg-primary text-primary-foreground hover:bg-primary" : "text-muted-foreground"
                        )}
                      >
                        {step} mm
                      </button>
                    ))}
                  </div>
                  <div className="font-mono text-[10px] text-muted-foreground">F{JOG_FEED} mm/min</div>
                  <Button
                    size="sm"
                    variant={controller.indicatorOn ? "warning" : "secondary"}
                    className="w-full"
                    disabled={!connected || controller.stream.printing || (!controller.indicatorOn && !canJog)}
                    onClick={() => void controller.setIndicator(!controller.indicatorOn)}
                  >
                    {controller.indicatorOn ? <EyeOff /> : <Eye />}
                    {controller.indicatorOn ? "Indicator off" : "Indicator 2%"}
                  </Button>
                </div>
              </div>
              <div className="grid grid-cols-3 gap-2">
                <Button variant="destructive" disabled={!canJog || coverOpen || !job?.stats.fitsKeycap} onClick={() => void startJob()}><Play />Start</Button>
                <Button variant="warning" disabled={!connected} onClick={() => void controller.reset()}><RotateCcw />Reset</Button>
                <Button disabled={!connected} onClick={() => void controller.disconnect()}><Unplug />Close</Button>
              </div>
              <Progress value={progress} />
              <div className="flex min-h-4 justify-between gap-4 font-mono text-[10px] text-muted-foreground">
                <span>{controller.stream.current} / {controller.stream.total}</span>
                <span className="truncate text-right">{controller.stream.line}</span>
              </div>
            </div>
          </section>

          <section className="min-h-[300px] overflow-hidden rounded-lg border border-border bg-card min-[1180px]:col-span-2">
            <Tabs defaultValue="raster" className="flex h-full min-h-[300px] flex-col">
              <div className="flex h-12 shrink-0 items-center border-b border-border px-3">
                <TabsList>
                  <TabsTrigger value="raster"><Route className="mr-1.5 size-3.5" />Raster</TabsTrigger>
                  <TabsTrigger value="console"><Activity className="mr-1.5 size-3.5" />Console</TabsTrigger>
                </TabsList>
              </div>
              <TabsContent value="raster" className="p-3">
                <textarea
                  value={rasterSummary}
                  readOnly
                  spellCheck={false}
                  aria-label="GRBL-M3 raster job summary"
                  className="h-full min-h-[230px] w-full resize-none rounded-md border border-border bg-code p-3 font-mono text-[11px] leading-5 text-code-foreground outline-none"
                />
              </TabsContent>
              <TabsContent value="console" className="p-3">
                <pre ref={consoleRef} className="h-full min-h-[230px] overflow-auto rounded-md border border-border bg-console p-3 font-mono text-[11px] leading-5 text-console-foreground">
                  {controller.logs.join("\n")}
                </pre>
              </TabsContent>
            </Tabs>
          </section>
        </div>
      </section>
    </main>
  );
}

function PanelTitle({ icon, title, trailing }: { icon: ReactNode; title: string; trailing?: string }) {
  return (
    <div className="flex h-12 shrink-0 items-center justify-between border-b border-border px-4">
      <div className="flex items-center gap-2 text-sm font-semibold [&_svg]:size-4 [&_svg]:text-muted-foreground">{icon}{title}</div>
      {trailing ? <span className="text-[11px] font-medium text-muted-foreground">{trailing}</span> : null}
    </div>
  );
}

function Metric({ label, value, compact, mono }: { label: string; value: ReactNode; compact?: boolean; mono?: boolean }) {
  return (
    <div className={cn("px-4 py-3", compact && "py-2.5")}>
      <div className={cn("text-sm font-semibold", mono && "font-mono text-xs")}>{value}</div>
      <div className="mt-0.5 text-[10px] font-medium uppercase text-muted-foreground">{label}</div>
    </div>
  );
}

function SettingsGroup({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="space-y-3.5">
      <div className="flex items-center gap-2 text-xs font-semibold uppercase text-muted-foreground">
        {title === "Laser" ? <Gauge className="size-3.5" /> : <SlidersHorizontal className="size-3.5" />}{title}
      </div>
      {children}
    </div>
  );
}

function NumberField({ label, value, min, max, step, onChange }: { label: string; value: number; min: number; max: number; step: number; onChange: (value: number) => void }) {
  const [draft, setDraft] = useState(String(value));

  useEffect(() => {
    setDraft(String(value));
  }, [value]);

  function commit() {
    const parsed = Number(draft);
    if (Number.isFinite(parsed)) onChange(parsed);
    else setDraft(String(value));
  }

  return (
    <label className="space-y-1.5 text-[11px] font-medium text-muted-foreground">
      <span>{label}</span>
      <Input
        type="number"
        value={draft}
        min={min}
        max={max}
        step={step}
        onChange={(event) => setDraft(event.target.value)}
        onBlur={commit}
        onKeyDown={(event) => { if (event.key === "Enter") event.currentTarget.blur(); }}
      />
    </label>
  );
}

function RangeField({ label, value, min, max, step, suffix, onChange }: { label: string; value: number; min: number; max: number; step: number; suffix: string; onChange: (value: number) => void }) {
  return (
    <div className="grid grid-cols-[70px_1fr_56px] items-center gap-2.5">
      <span className="text-[11px] font-medium text-muted-foreground">{label}</span>
      <Slider value={[value]} min={min} max={max} step={step} onValueChange={([next]) => onChange(next)} />
      <span className="text-right font-mono text-[11px] font-medium">{value.toFixed(step < 1 ? 1 : 0)} {suffix}</span>
    </div>
  );
}

function CheckField({ label, checked, onChange, danger, disabled }: { label: string; checked: boolean; onChange: (checked: boolean) => void; danger?: boolean; disabled?: boolean }) {
  return (
    <label className={cn("flex h-9 items-center gap-2 rounded-md border border-border bg-muted/30 px-2.5 text-[11px] font-medium", danger && checked && "border-destructive/30 bg-destructive/10 text-destructive", disabled && "cursor-not-allowed opacity-50")}>
      <Checkbox disabled={disabled} checked={checked} onCheckedChange={(value) => onChange(value === true)} className={danger ? "data-[state=checked]:border-destructive data-[state=checked]:bg-destructive" : undefined} />
      {label}
    </label>
  );
}

function isSimpleIcon(value: unknown): value is SimpleIcon {
  if (!value || typeof value !== "object") return false;
  const icon = value as Partial<SimpleIcon>;
  return typeof icon.title === "string" && typeof icon.slug === "string" && typeof icon.path === "string";
}

function iconLabel(key: string): string {
  return key.replace(/([a-z0-9])([A-Z])/g, "$1-$2").replace(/([A-Z])([A-Z][a-z])/g, "$1-$2").replace(/_/g, "-").toLowerCase();
}
