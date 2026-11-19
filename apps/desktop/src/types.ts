// Mirrors docs/ui-contract.md exactly.

export type Setup = "starting" | "ready" | "installBoat" | "signInBoat" | "installT3";

export type MachineState =
  | "working"
  | "idle"
  | "sleeping"
  | "asleep"
  | "waking"
  | "creating"
  | "error"
  | "missing"
  | "unknown";

export interface MachineView {
  id: string;
  name: string;
  size: string;
  account: string | null; // the settings' Boat account it lives in; null: the Boat CLI's login
  state: MachineState;
  detail: string;
  countdown: Countdown | null; // set while `detail` counts down
  paired: boolean;
  signedOut: string[];
  projects: MachineProject[]; // its T3 projects as last seen (never wakes it), then those being added
  keepAwakeUntil: number | null; // when a "keep awake" runs out (unix ms); null when off or until stopped
  keepAwakeForever: boolean; // kept awake until someone stops it: no auto sleep
}

export interface MachineProject {
  id: string; // T3's project id on the machine; while t3boat adds it, the local project's id
  title: string;
  repoKey: string | null; // T3's repository key: the same repo has it on every machine
  status: string | null; // set while t3boat adds it: clone progress as T3 Code words it
}

/** A T3 Code project on this computer that can go to a machine (it has a git remote). */
export interface LocalProject {
  id: string;
  title: string;
  path: string;
  repoKey: string;
  icon: string | null; // data: URL of the project's favicon in T3 Code
}

/** `detail` as `template` with `{left}` filled in: the time left until `endsAt` (unix ms). */
export interface Countdown {
  template: string;
  endsAt: number;
}

export interface Settings {
  startAtLogin: boolean;
  boatAccounts: BoatAccount[];
}

/** A saved Boat snapshot a machine can start from. */
export interface Template {
  name: string;
  createdAt: string | null;
  sizeBytes: number | null; // Boat reports it for some snapshots only
}

/** A Boat account besides the CLI's own login. Holds where its key is, never the key. */
export interface BoatAccount {
  name: string;
  keychain?: string; // keychain service of a generic password holding a Boat API key
  org?: string; // organization the sandboxes bill to
}

export interface Size {
  id: string;
  spec: string;
  hourly: number;
}

export type McpClient = "claudeCode" | "codex";

export interface UiState {
  setup: Setup;
  setupBusy: boolean;
  /** while a Boat sign-in waits: the code its page may ask for */
  signInCode: string | null;
  account: { identifier: string | null; plan: string | null } | null;
  machines: MachineView[];
  costs: { today: number; month: number; startsToday: number; startsLimit: number };
  settings: Settings;
  sizes: Size[];
  /** null = that CLI isn't installed */
  mcp: Record<McpClient, boolean | null>;
  /** "server": t3boat-server's web UI, whose browser is on another computer; absent in older shells */
  host?: "desktop" | "server";
}

/** A fresh pairing: the t3code:// link, the localhost URL it carries, and the port it needs. */
export interface Pairing {
  link: string;
  pairingUrl: string;
  port: number;
}

/** Where "Open" goes, from t3boat-server: a T3 Code link, or a pairing for an unpaired machine. */
export type T3Link = { kind: "open"; url: string } | ({ kind: "pair" } & Pairing);

/** A sign-in with no local callback (t3boat-server): the page, and Codex's code to enter there. */
export interface ManualSignIn {
  url: string;
  userCode: string | null;
  /** the page gives a code to send back with sign_in_code (Claude) */
  needsCode: boolean;
}

export interface MachineDetails {
  id: string;
  costToday: number;
  costMonth: number;
  awakeSecsToday: number;
  agents: { name: string; signedIn: boolean }[];
  t3Version: string;
  createdAt: number; // unix secs
  /** T3 Connect is on: the user's other devices reach it with their T3 account */
  connect: boolean;
}

export type SetupAction = "installBoat" | "signInBoat" | "installT3" | "recheck";

export interface Commands {
  get_state: [undefined, UiState];
  create_machine: [{ name: string; size: string; account?: string; template?: string; projects?: string[] }, void]; // account: a settings Boat account's name; template: saved Boat snapshot; projects: LocalProject ids to add once up
  templates: [{ account?: string }, Template[]]; // saved Boat snapshots a machine can start from, in that account, newest first
  local_projects: [undefined, LocalProject[]]; // pinned first, then most recent
  add_project: [{ id: string; projectId: string }, void]; // resolves once cloned
  wake_machine: [{ id: string }, void];
  sleep_machine: [{ id: string }, void];
  open_machine: [{ id: string; thread?: string }, void | T3Link]; // T3Link from t3boat-server
  pair_machine: [{ id: string }, void | Pairing]; // Pairing from t3boat-server
  delete_machine: [{ id: string; discard?: boolean }, void];
  resize_machine: [{ id: string; size: string }, void];
  keep_awake: [{ id: string; minutes: number | null }, void]; // null: until stopped; 0: stop
  sign_in_agent: [{ id: string; agent: string }, void | ManualSignIn]; // ManualSignIn from t3boat-server, which finishes in the background
  sign_in_code: [{ id: string; agent: string; code: string }, void]; // t3boat-server only
  turn_on_connect: [{ id: string }, void]; // resolves once reachable, after the browser sign-in
  latest_thread: [{ id: string }, string | null]; // thread id
  get_machine_details: [{ id: string }, MachineDetails];
  add_mcp: [{ client: McpClient }, void];
  update_settings: [{ settings: Settings }, void];
  setup_action: [{ action: SetupAction }, void];
  quit_app: [undefined, void];
}

export type CommandName = keyof Commands;
export type CommandArgs<C extends CommandName> = Commands[C][0];
export type CommandResult<C extends CommandName> = Commands[C][1];
