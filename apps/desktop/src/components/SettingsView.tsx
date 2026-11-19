import { useEffect, useState } from "react";
import type { BoatAccount, McpClient, Settings, UiState } from "../types";
import { AgentName, AgentNames } from "./AgentLogos";
import { Icon } from "./Icons";
import { Dots } from "./common";
import { SubHeader } from "./SubHeader";

const CLIENTS: { id: McpClient; name: string }[] = [
  { id: "claudeCode", name: "Claude Code" },
  { id: "codex", name: "Codex" },
];

interface Props {
  state: UiState;
  onSave: (s: Settings) => void;
  /** never rejects; errors are shown by the caller */
  onAddMcp: (client: McpClient) => Promise<void>;
  onQuit: () => void;
  onBack: () => void;
}

export function SettingsView({ state, onSave, onAddMcp, onQuit, onBack }: Props) {
  // optimistic copy; the next "state" event is the source of truth
  const [s, setS] = useState(state.settings);
  useEffect(() => setS(state.settings), [state.settings]);

  // hidden by default so screen recordings don't leak the email
  const [showEmail, setShowEmail] = useState(false);
  const email = state.account?.identifier;
  const plan = state.account?.plan;

  const [adding, setAdding] = useState<McpClient | null>(null);
  const add = (client: McpClient) => {
    setAdding(client);
    onAddMcp(client).finally(() => setAdding(null));
  };
  // t3boat-server: no login to start at, no local coding agents to add, nothing to quit
  const server = state.host === "server";
  const missing = CLIENTS.filter((c) => state.mcp[c.id] === null).map((c) => c.name);

  const change = (p: Partial<Settings>) => {
    const next = { ...s, ...p };
    setS(next);
    onSave(next);
  };

  return (
    <div className="view">
      <SubHeader title="Settings" onBack={onBack} />
      <div className="view-body settings">
        <p className="hint">
          Machines sleep 5 minutes after their agents finish and wake by themselves when you use them in T3 Code.
        </p>
        {server ? null : (
        <label className="setting setting-row">
          <span className="setting-name">Start at login</span>
          <input
            type="checkbox"
            role="switch"
            className="switch"
            checked={s.startAtLogin}
            onChange={(e) => change({ startAtLogin: e.target.checked })}
          />
        </label>
        )}
        {server ? null : (
        <div className="setting">
          <span className="caption">mcp</span>
          <span className="setting-name">
            Let <AgentNames names={CLIENTS.map((c) => c.name)} /> control t3boat
          </span>
          <div className="mcp-buttons">
            {CLIENTS.map((c) => {
              const added = state.mcp[c.id];
              return added ? (
                <span key={c.id} className="mcp-added">
                  <Icon name="check" size={10} />
                  <span>
                    Added to <AgentName name={c.name} />
                  </span>
                </span>
              ) : (
                <button
                  key={c.id}
                  className="btn btn-sm btn-outline"
                  disabled={added === null || adding !== null}
                  aria-busy={adding === c.id}
                  onClick={() => add(c.id)}
                >
                  {adding === c.id ? (
                    <>
                      <Dots /> Adding…
                    </>
                  ) : (
                    <span>
                      Add to <AgentName name={c.name} />
                    </span>
                  )}
                </button>
              );
            })}
          </div>
          <span className="hint">
            {missing.length ? (
              <>
                <AgentNames names={missing} /> {missing.length === 1 ? "isn't" : "aren't"} installed on this computer
              </>
            ) : (
              "Their agents can then create, wake and send work to machines"
            )}
          </span>
        </div>
        )}
        <BoatAccounts accounts={s.boatAccounts} onChange={(boatAccounts) => change({ boatAccounts })} />
      </div>
      <div className="view-foot settings-foot">
        <span className="setting-value" title="Boat account">
          {email && (
            <button className="account-email" onClick={() => setShowEmail((v) => !v)}>
              {showEmail ? email : "Show email"}
            </button>
          )}
          {email && plan && " · "}
          {plan}
        </span>
        {server ? null : (
          <button className="btn btn-sm btn-quiet" onClick={onQuit}>
            Quit t3boat
          </button>
        )}
      </div>
    </div>
  );
}

const ACCOUNT_WORD = /^[A-Za-z0-9][A-Za-z0-9._-]*$/;

function BoatAccounts({ accounts, onChange }: { accounts: BoatAccount[]; onChange: (a: BoatAccount[]) => void }) {
  const [draft, setDraft] = useState({ name: "", keychain: "", org: "" });
  const values = [draft.name, draft.keychain, draft.org].filter(Boolean);
  const problem = !draft.name
    ? null
    : accounts.some((a) => a.name === draft.name)
      ? "There is an account with this name"
      : !values.every((v) => ACCOUNT_WORD.test(v))
        ? "Letters, digits, dots, dashes and underscores only"
        : !draft.keychain && !draft.org
          ? "Give a keychain item, an organization or both"
          : null;
  const add = () => {
    if (!draft.name || problem) return;
    onChange([...accounts, { name: draft.name, keychain: draft.keychain || undefined, org: draft.org || undefined }]);
    setDraft({ name: "", keychain: "", org: "" });
  };
  const field = (key: keyof typeof draft, placeholder: string, label: string) => (
    <input
      aria-label={label}
      className="account-input"
      placeholder={placeholder}
      value={draft[key]}
      onChange={(e) => setDraft({ ...draft, [key]: e.target.value.trim() })}
      onKeyDown={(e) => e.key === "Enter" && add()}
      spellCheck={false}
      autoComplete="off"
    />
  );
  return (
    <div className="setting">
      <span className="caption">boat accounts</span>
      <span className="setting-name">Create machines in other Boat accounts</span>
      {accounts.map((a) => (
        <div key={a.name} className="boat-account">
          <span>
            <span className="setting-name">{a.name}</span>
            <span className="setting-value">{[a.keychain && `keychain ${a.keychain}`, a.org && `org ${a.org}`].filter(Boolean).join(" · ")}</span>
          </span>
          <button className="btn btn-sm btn-quiet" onClick={() => onChange(accounts.filter((x) => x.name !== a.name))}>
            Remove
          </button>
        </div>
      ))}
      <div className="account-form">
        {field("name", "name", "Account name")}
        {field("keychain", "keychain", "Keychain service of the Boat API key")}
        {field("org", "org", "Boat organization")}
        <button className="btn btn-sm btn-outline" disabled={!draft.name || !!problem} onClick={add}>
          Add
        </button>
      </div>
      <span className={`hint${problem ? " is-error" : ""}`}>
        {problem ?? "The API key stays in your keychain; t3boat keeps only the item's service name"}
      </span>
    </div>
  );
}
