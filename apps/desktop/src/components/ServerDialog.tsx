import { useEffect, useRef, useState } from "react";
import { call, errorText } from "../api";
import type { ManualSignIn, Pairing } from "../types";

/** What t3boat-server hands back for a browser on another computer to finish. */
export type ServerTask =
  | ({ kind: "pair"; machine: string } & Pairing)
  | ({ kind: "signIn"; machine: string; machineId: string; agent: string } & ManualSignIn);

interface Props {
  task: ServerTask;
  onClose: () => void;
  onDone: (text: string) => void;
}

/** A modal for the steps the web UI's person does themselves: pairing T3 Code over an SSH
 *  forward, or a sign-in finished on the vendor's page. */
export function ServerDialog({ task, onClose, onDone }: Props) {
  const dialog = useRef<HTMLDialogElement>(null);
  // The button that asked for this: focus goes back to it when the dialog closes.
  const opener = useRef(document.activeElement instanceof HTMLElement ? document.activeElement : null);
  useEffect(() => {
    const d = dialog.current;
    if (d && !d.open) d.showModal();
  }, []);
  const close = () => {
    // The opener showed "Sending…" while the command ran and may have dropped focus: then the
    // machine's own row takes it.
    const own = opener.current && opener.current !== document.body ? opener.current : null;
    const back = own ?? document.querySelector<HTMLElement>(`[aria-label="Details for ${CSS.escape(task.machine)}"]`);
    onClose();
    if (back?.isConnected) requestAnimationFrame(() => back.focus());
  };

  return (
    <dialog ref={dialog} className="server-dialog" onClose={close} aria-labelledby="server-dialog-title">
      {task.kind === "pair" ? <PairBody task={task} /> : <SignInBody task={task} onDone={onDone} />}
      <div className="server-dialog-foot">
        <button className="btn btn-sm btn-quiet" onClick={() => dialog.current?.close()}>
          Close
        </button>
      </div>
    </dialog>
  );
}

function PairBody({ task }: { task: Extract<ServerTask, { kind: "pair" }> }) {
  const forward = `ssh -N -L ${task.port}:localhost:${task.port} ${task.sshHost ?? "<this server>"}`;
  return (
    <>
      <h2 id="server-dialog-title" className="server-dialog-title">
        Add {task.machine} to T3 Code
      </h2>
      <ol className="server-dialog-steps">
        <li>
          <span>On the computer running T3 Code, forward the machine's port from this server:</span>
          <Copyable value={forward} />
        </li>
        <li>
          <span>Then open the link there and accept the new backend in T3 Code.</span>
          <a className="btn btn-sm btn-solid" href={task.link}>
            Open in T3 Code
          </a>
        </li>
      </ol>
    </>
  );
}

function SignInBody({ task, onDone }: { task: Extract<ServerTask, { kind: "signIn" }>; onDone: (t: string) => void }) {
  const [code, setCode] = useState("");
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const send = (e: React.FormEvent) => {
    e.preventDefault();
    if (!code.trim()) {
      setError("Paste the code the page shows");
      return;
    }
    setSending(true);
    setError(null);
    call("sign_in_code", { id: task.machineId, agent: task.agent, code: code.trim() }).then(
      () => onDone(`${task.agent} is signing in on ${task.machine}`),
      (err) => {
        setSending(false);
        setError(errorText(err));
      },
    );
  };
  return (
    <>
      <h2 id="server-dialog-title" className="server-dialog-title">
        Sign {task.agent} in on {task.machine}
      </h2>
      <ol className="server-dialog-steps">
        <li>
          <span>Open the sign-in page and authorize.</span>
          <a className="btn btn-sm btn-solid" href={task.url} target="_blank" rel="noreferrer">
            Open sign-in page
          </a>
        </li>
        {task.userCode ? (
          <li>
            <span>Enter this one-time code there. The machine finds out by itself when you're done.</span>
            <Copyable value={task.userCode} />
          </li>
        ) : null}
        {task.needsCode ? (
          <li>
            <form className="server-dialog-form" onSubmit={send} noValidate>
              <label htmlFor="sign-in-code">Code from the page</label>
              <div className="server-dialog-row">
                <input
                  id="sign-in-code"
                  className="account-input"
                  value={code}
                  autoComplete="one-time-code"
                  autoCapitalize="none"
                  spellCheck={false}
                  required
                  aria-invalid={error ? true : undefined}
                  aria-describedby={error ? "sign-in-code-error" : undefined}
                  onChange={(e) => setCode(e.target.value)}
                />
                <button type="submit" className="btn btn-sm btn-solid" disabled={sending}>
                  {sending ? "Sending…" : "Send"}
                </button>
              </div>
              {error ? (
                <span id="sign-in-code-error" className="hint is-error">
                  {error}
                </span>
              ) : null}
            </form>
          </li>
        ) : null}
      </ol>
    </>
  );
}

/** A value to paste elsewhere: selectable whole, with a copy button that works on plain http too. */
function Copyable({ value }: { value: string }) {
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(value);
    } catch {
      // http on the tailnet has no clipboard API: select the text for Ctrl+C instead
      const range = document.createRange();
      const el = document.getElementById(`copy-${btoa(value).slice(0, 12)}`);
      if (el) {
        range.selectNodeContents(el);
        getSelection()?.removeAllRanges();
        getSelection()?.addRange(range);
      }
      return;
    }
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };
  return (
    <div className="server-dialog-row">
      <code id={`copy-${btoa(value).slice(0, 12)}`} className="copyable">
        {value}
      </code>
      <button className="btn btn-sm btn-outline" onClick={copy} aria-label={`Copy ${value}`}>
        {copied ? "Copied" : "Copy"}
      </button>
    </div>
  );
}
