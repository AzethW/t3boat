import { useEffect, useRef, useState, type FormEvent, type KeyboardEvent } from "react";
import type { UiState } from "../types";
import { call } from "../api";
import { cleanName, hourly, NAME_RE, suggestName } from "../machine";
import { Icon } from "./Icons";
import { ProjectList, useLocalProjects } from "./ProjectPicker";
import { SubHeader } from "./SubHeader";

interface Props {
  state: UiState;
  onNameChange: (name: string) => void;
  onCreate: (name: string, size: string, account: string, template: string, projects: string[]) => void;
  onBack: () => void;
}

export function NewMachine({ state, onNameChange, onCreate, onBack }: Props) {
  const taken = state.machines.map((m) => m.name);
  const [name, setName] = useState(() => suggestName(taken));
  // Boat's own default: room for builds and tests without the large price.
  const [size, setSize] = useState(() => (state.sizes.some((s) => s.id === "default") ? "default" : (state.sizes[0]?.id ?? "")));
  const input = useRef<HTMLInputElement>(null);
  const projects = useLocalProjects();
  const [picked, setPicked] = useState<string[]>([]);
  const toggle = (id: string) => setPicked((p) => (p.includes(id) ? p.filter((x) => x !== id) : [...p, id]));
  const accounts = state.settings.boatAccounts;
  const [account, setAccount] = useState("");
  const [template, setTemplate] = useState("");
  const templates = useTemplates(account);

  useEffect(() => {
    input.current?.select();
  }, []);
  useEffect(() => onNameChange(name), [name, onNameChange]);

  const problem = !name
    ? "Give it a name"
    : !NAME_RE.test(name)
      ? "Start with a letter or number"
      : taken.includes(name)
        ? "You already have a machine with this name"
        : null;

  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (!problem && size) onCreate(name, size, account, template.trim(), picked);
  };

  return (
    <form className="view" onSubmit={submit}>
      <SubHeader title="New machine" onBack={onBack} />
      <div className="view-body">
        <div className="field">
          <label className="caption" htmlFor="machine-name">
            name
          </label>
          <span className="prompt-input">
            <span className="prompt-sign" aria-hidden>
              $
            </span>
            <input
              id="machine-name"
              ref={input}
              aria-describedby="machine-name-hint"
              value={name}
              onChange={(e) => setName(cleanName(e.target.value))}
              spellCheck={false}
              autoComplete="off"
              aria-invalid={!!problem}
            />
          </span>
          <span id="machine-name-hint" className={`hint${problem && name ? " is-error" : ""}`}>
            {problem ?? "Shows up under this name in T3 Code"}
          </span>
        </div>

        <fieldset className="field">
          <legend className="caption">size</legend>
          <div className="sizes" role="radiogroup">
            {state.sizes.map((s) => (
              <label key={s.id} className={`size${size === s.id ? " is-on" : ""}`}>
                <input type="radio" name="size" value={s.id} checked={size === s.id} onChange={() => setSize(s.id)} />
                <span className="size-name">{s.id}</span>
                <span className="size-spec">{s.spec}</span>
                <span className="size-price">{hourly(s.hourly)}</span>
              </label>
            ))}
          </div>
          <span className="hint">
            Billed only while awake · sleeps 5 min after agents finish · resize anytime
          </span>
        </fieldset>

        {accounts.length > 0 && (
          <div className="field">
            <label className="caption" htmlFor="machine-account">
              boat account
            </label>
            <select id="machine-account" className="account-select" value={account} onChange={(e) => setAccount(e.target.value)}>
              <option value="">Boat CLI login</option>
              {accounts.map((a) => (
                <option key={a.name} value={a.name}>
                  {a.name}
                </option>
              ))}
            </select>
            <span className="hint">Its sandbox, costs and snapshots stay in this account</span>
          </div>
        )}

        <div className="field">
          <label className="caption" htmlFor="machine-template">
            template
          </label>
          <span className="prompt-input">
            <span className="prompt-sign" aria-hidden>
              $
            </span>
            <input
              id="machine-template"
              aria-describedby="machine-template-hint"
              list="machine-templates"
              value={template}
              placeholder="none"
              onChange={(e) => setTemplate(e.target.value.replace(/\s/g, ""))}
              spellCheck={false}
              autoComplete="off"
            />
          </span>
          <datalist id="machine-templates">
            {templates.map((t) => (
              <option key={t} value={t} />
            ))}
          </datalist>
          <span id="machine-template-hint" className="hint">
            Optional · a saved Boat snapshot the machine starts from, your tools already on it
          </span>
        </div>

        <fieldset className="field">
          <legend className="caption">projects</legend>
          <div className="pick pick-checklist" role="group" aria-label="Projects to add">
            <div className="pick-scroll">
              <ProjectList
                projects={projects}
                shown={projects.list ?? []}
                query=""
                option={(p) => {
                  const on = picked.includes(p.id);
                  // One clone per repository: another project of the same repo is already picked.
                  const sameRepo = !on && (projects.list ?? []).some((o) => o.repoKey === p.repoKey && picked.includes(o.id));
                  const flip = () => !sameRepo && toggle(p.id);
                  return {
                    role: "checkbox",
                    tabIndex: 0,
                    "aria-checked": on,
                    "aria-disabled": sameRepo,
                    className: `pick-option${on ? " is-active" : ""}${sameRepo ? " is-done" : ""}`,
                    onClick: flip,
                    onKeyDown: (e: KeyboardEvent) => {
                      if (e.key === " " || e.key === "Enter") {
                        e.preventDefault();
                        flip();
                      }
                    },
                    badge: on ? (
                      <span className="line-ok">
                        <Icon name="check" size={10} />
                      </span>
                    ) : null,
                  };
                }}
              />
            </div>
          </div>
          <span className="hint">Optional · cloned from git once it's up, .env files included</span>
        </fieldset>
      </div>
      <div className="view-foot">
        <button type="submit" className="btn btn-solid btn-lg" disabled={!!problem || !size}>
          Launch machine
        </button>
        <p className="foot-note">
          Takes about a minute{picked.length ? `, then ${picked.length} ${picked.length === 1 ? "project" : "projects"}` : ""}. T3 Code
          will ask to add it.
        </p>
      </div>
    </form>
  );
}

/** Saved Boat snapshots, as suggestions: the field takes any name, so a failed lookup shows none. */
function useTemplates(account: string): string[] {
  const [list, setList] = useState<string[]>([]);
  useEffect(() => {
    let live = true;
    setList([]);
    call("templates", { account: account || undefined }).then(
      (t) => live && setList(t),
      () => {},
    );
    return () => {
      live = false;
    };
  }, [account]);
  return list;
}
