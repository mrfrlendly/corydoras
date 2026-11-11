import { TextareaHTMLAttributes, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { api, errText, fmtDate, fmtMeasure, PRINT_OUTCOMES, PrintOutcome, PrintResult, Project, Rule, RuleFeedback, RuleSet, RuleSetInput, RuleUnit } from '../api';
import { ArrowDown, ArrowUp, Check, Eye, Plus, Printer, Trash2, TriangleAlert, X } from 'lucide-react';
import { useToast } from './Toast';
import { useConfirmRow } from './useConfirmRow';

const UNITS: RuleUnit[] = ['mm', 'deg', 'percent', 'count', 'ratio'];

interface Props {
  ruleSet: RuleSet;
  projects: Project[];
  onChanged: () => void;
  onDeleted: () => void;
  onOpenProject: (id: string) => void;
}

function toInput(r: RuleSet): RuleSetInput {
  return { name: r.name, description: r.description, always: r.always, rules: r.rules.map((x) => ({ ...x })) };
}

/** A text area that grows with its text. */
function AutoArea(props: TextareaHTMLAttributes<HTMLTextAreaElement>) {
  const ref = useRef<HTMLTextAreaElement>(null);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = `${el.scrollHeight + 2}px`;
  }, [props.value]);
  return <textarea ref={ref} rows={1} {...props} />;
}

/** A number field that keeps partial input such as "4." or "-" while you type. */
function NumField({ value, onValue, label }: { value: number | null; onValue: (v: number | null) => void; label?: string }) {
  const [text, setText] = useState(value === null ? '' : String(value));
  useEffect(() => {
    // Follow outside changes (discard, reload) without fighting the typing.
    const parsed = text.trim() === '' ? null : Number(text.replace(',', '.'));
    if (parsed !== value && !(parsed !== null && Number.isNaN(parsed))) {
      setText(value === null ? '' : String(value));
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value]);
  return (
    <input
      className="num"
      type="text"
      inputMode="decimal"
      placeholder="—"
      aria-label={label}
      value={text}
      onChange={(e) => {
        const t = e.target.value;
        setText(t);
        if (t.trim() === '') return onValue(null);
        const n = Number(t.replace(',', '.'));
        if (Number.isFinite(n)) onValue(n);
      }}
      onBlur={() => setText(value === null ? '' : String(value))}
    />
  );
}

function blankRule(): Rule {
  return { name: '', value: null, unit: null, note: '' };
}

/** The print log of one rule set: what real prints showed, and where they disagree with a limit. */
function PrintLogPanel({ ruleSet }: { ruleSet: RuleSet }) {
  const toast = useToast();
  const [log, setLog] = useState<PrintResult[]>([]);
  const [feedback, setFeedback] = useState<RuleFeedback[]>([]);
  const [adding, setAdding] = useState(false);
  const [result, setResult] = useState<PrintOutcome>('failed');
  const [material, setMaterial] = useState('');
  const [rule, setRule] = useState('');
  const [value, setValue] = useState<number | null>(null);
  const [note, setNote] = useState('');
  const [confirmRemove, setConfirmRemove] = useState<string | null>(null);
  // The row whose Remove was asked last; focus goes back to its button.
  const [asked, setAsked] = useState<string | null>(null);
  const removeTrigger = useConfirmRow(confirmRemove !== null, () => setConfirmRemove(null));

  const load = () => {
    api
      .listPrintResults()
      .then((all) => setLog(all.filter((p) => p.rule_set_id === ruleSet.id || p.rule_set_id === null)))
      .catch((e) => toast.error(errText(e)));
    api
      .ruleFeedback(ruleSet.id)
      .then(setFeedback)
      .catch(() => setFeedback([]));
  };
  useEffect(load, [ruleSet.id, ruleSet.updated_at]); // eslint-disable-line react-hooks/exhaustive-deps

  const limits = ruleSet.rules.filter((r) => r.value !== null);
  const unitOf = (name: string) => ruleSet.rules.find((r) => r.name === name)?.unit ?? null;

  const add = async () => {
    try {
      await api.addPrintResult({
        result,
        material,
        rule,
        rule_set: ruleSet.id,
        value: rule ? value : null,
        unit: rule && value !== null ? unitOf(rule) : null,
        note,
        language: null,
        output: null,
      });
      setAdding(false);
      setNote('');
      setValue(null);
      load();
    } catch (e) {
      toast.error(errText(e));
    }
  };

  const hints = feedback.flatMap((f) => f.hints.map((h) => ({ rule: f.rule, text: h })));

  return (
    <section className="panel">
      <div className="row" style={{ marginTop: 0 }}>
        <h3 className="grow" style={{ marginBottom: 0 }}>
          <Printer size={15} /> Print feedback
        </h3>
        {!adding && (
          <button className="btn small" onClick={() => setAdding(true)}>
            <Plus size={13} /> Print result
          </button>
        )}
      </div>
      <p className="muted small">
        What happened when a design was printed. Agents get these results with the hard rules, and the conformance check warns when a new design is at
        or past a value that failed. An agent can add a result when you tell it how a print came out.
      </p>
      {hints.map((h, i) => (
        <div key={i} className="warn-box">
          <TriangleAlert size={14} /> <strong>{h.rule}:</strong> {h.text}
        </div>
      ))}
      {adding && (
        <div className="print-form">
          <select className="select small" value={result} onChange={(e) => setResult(e.target.value as PrintOutcome)} aria-label="Result of the print">
            {PRINT_OUTCOMES.map((r) => (
              <option key={r} value={r}>
                {r}
              </option>
            ))}
          </select>
          <input className="small-input" placeholder="Material" aria-label="Material" value={material} onChange={(e) => setMaterial(e.target.value)} maxLength={120} />
          <select className="select small" value={rule} onChange={(e) => setRule(e.target.value)} aria-label="Rule">
            <option value="">No rule (about the material)</option>
            {limits.map((r) => (
              <option key={r.name} value={r.name}>
                {r.name}
              </option>
            ))}
          </select>
          {rule && (
            <label className="small muted up-pick" title="The value that was printed, for example a wall of 1.0 mm">
              Printed at
              <NumField value={value} onValue={setValue} /> {unitOf(rule) ?? ''}
            </label>
          )}
          <input className="output-note-input" placeholder="What you saw" aria-label="What you saw" value={note} onChange={(e) => setNote(e.target.value)} maxLength={400} />
          <button className="btn small primary" onClick={add} disabled={!material.trim() && !rule}>
            Add result
          </button>
          <button className="btn small" onClick={() => setAdding(false)}>
            Cancel
          </button>
        </div>
      )}
      {log.length === 0 ? (
        <p className="muted small">No print results yet.</p>
      ) : (
        <table className="vtable print-table">
          <thead>
            <tr>
              <th>Result</th>
              <th>Material</th>
              <th>Rule</th>
              <th>Printed at</th>
              <th>Note</th>
              <th>When</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {log.map((p) => (
              <tr key={p.id}>
                <td>
                  <span className={`print-result print-${p.result}`}>{p.result}</span>
                </td>
                <td>{p.material || '—'}</td>
                <td>{p.rule || '—'}</td>
                <td className="mono small">{p.value !== null ? fmtMeasure(p.value, p.unit) : '—'}</td>
                <td className="small">
                  {p.note}
                  {p.rule_set_id === null && <span className="tag-pill">applies everywhere</span>}
                  {p.author && <span className="tag-pill">{p.author}</span>}
                </td>
                <td className="small muted">{fmtDate(p.created_at)}</td>
                <td>
                  <div className="version-tools">
                    {confirmRemove === p.id ? (
                      <>
                        <button
                          className="btn danger small"
                          onClick={() =>
                            api
                              .removePrintResult(p.id)
                              .then(() => {
                                setConfirmRemove(null);
                                load();
                              })
                              .catch((e) => toast.error(errText(e)))
                          }
                        >
                          Remove
                        </button>
                        <button className="btn small" autoFocus onClick={() => setConfirmRemove(null)}>
                          Keep
                        </button>
                      </>
                    ) : (
                      <button
                        ref={asked === p.id ? removeTrigger : undefined}
                        className="icon-btn"
                        title="Remove this result…"
                        aria-label="Remove this result"
                        onClick={() => {
                          setAsked(p.id);
                          setConfirmRemove(p.id);
                        }}
                      >
                        <X size={14} />
                      </button>
                    )}
                  </div>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}

export default function RuleSetView({ ruleSet, projects, onChanged, onDeleted, onOpenProject }: Props) {
  const toast = useToast();
  const [draft, setDraft] = useState<RuleSetInput>(toInput(ruleSet));
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [preview, setPreview] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const deleteTrigger = useConfirmRow(confirmDelete, () => setConfirmDelete(false));

  // The library reloads after every queue job and gives a new object each time.
  // Follow the saved rule set only when it changes, so a rule being typed stays.
  const server = JSON.stringify(toInput(ruleSet));
  useEffect(() => {
    setDraft(JSON.parse(server) as RuleSetInput);
    setPreview(null);
  }, [server]);

  const dirty = JSON.stringify(draft) !== server;
  const users = projects.filter((p) => p.rule_set_ids.includes(ruleSet.id));

  const setRule = (i: number, patch: Partial<Rule>) =>
    setDraft((d) => ({ ...d, rules: d.rules.map((r, k) => (k === i ? { ...r, ...patch } : r)) }));

  const move = (i: number, dir: -1 | 1) =>
    setDraft((d) => {
      const j = i + dir;
      if (j < 0 || j >= d.rules.length) return d;
      const rules = [...d.rules];
      [rules[i], rules[j]] = [rules[j], rules[i]];
      return { ...d, rules };
    });

  const save = async () => {
    setSaving(true);
    try {
      // The library trims the text and drops empty rules. Show what it saved, so the draft is not dirty after a save.
      const saved = await api.updateRuleSet(ruleSet.id, draft);
      setDraft(toInput(saved));
      toast.info('Rule set saved.');
      onChanged();
    } catch (e) {
      toast.error(errText(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="settings">
      <header className="lang-head" data-tauri-drag-region>
        <div className="lang-title">
          <div className="eyebrow">Hard rule set</div>
          <input
            className="title-input plain"
            value={draft.name}
            onChange={(e) => setDraft({ ...draft, name: e.target.value })}
            placeholder="Rule set name"
          />
          <p className="lang-desc muted">Hard rules win over every design language. Agents get them first, before any language.</p>
        </div>
        <div className="lang-actions">
          {dirty && (
            <>
              <button className="btn small" onClick={() => setDraft(toInput(ruleSet))} disabled={saving}>
                Discard
              </button>
              <button className="btn small primary" onClick={save} disabled={saving || !draft.name.trim()}>
                <Check size={13} /> Save rule set
              </button>
            </>
          )}
        </div>
      </header>

      <div className="settings-body">
        <section className="panel">
          <label className="field">
            <span>Description</span>
            <textarea
              rows={2}
              value={draft.description}
              onChange={(e) => setDraft({ ...draft, description: e.target.value })}
              placeholder="What these rules cover, for example the printer, material and nozzle."
            />
          </label>
          <label className="check">
            <input type="checkbox" checked={draft.always} onChange={(e) => setDraft({ ...draft, always: e.target.checked })} />
            Apply to every project, and to agents that read a language without a project
          </label>
          <p className="muted small" style={{ marginTop: 10 }}>
            {draft.always
              ? 'This rule set applies everywhere.'
              : users.length === 0
                ? 'No project selects this rule set yet. Select it in a project.'
                : 'Selected by: '}
            {!draft.always &&
              users.map((p, i) => (
                <span key={p.id}>
                  {i > 0 ? ', ' : ''}
                  <button className="link-btn" onClick={() => onOpenProject(p.id)}>
                    {p.name}
                  </button>
                </span>
              ))}
          </p>
        </section>

        <section className="panel">
          <h3>Rules</h3>
          <p className="muted small">
            A rule with a value is a limit, for example a minimum wall of 1.2 mm. A rule without a value is an instruction. When a language
            marks a measure process-dependent, agents take the value from the rule with the same name.
          </p>
          <table className="ctable rtable">
            <thead>
              <tr>
                <th>Rule</th>
                <th>Value</th>
                <th>Unit</th>
                <th>Details</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {draft.rules.map((r, i) => (
                <tr key={i}>
                  <td>
                    <input value={r.name} onChange={(e) => setRule(i, { name: e.target.value })} placeholder="Minimum wall" aria-label="Rule name" />
                  </td>
                  <td>
                    <NumField label="Value" value={r.value} onValue={(v) => setRule(i, { value: v, unit: v === null ? null : r.unit ?? 'mm' })} />
                  </td>
                  <td>
                    <select
                      className="select small"
                      value={r.unit ?? ''}
                      disabled={r.value === null}
                      onChange={(e) => setRule(i, { unit: (e.target.value || null) as RuleUnit | null })}
                      aria-label="Unit"
                    >
                      {r.value === null && <option value="">—</option>}
                      {UNITS.map((u) => (
                        <option key={u} value={u}>
                          {u}
                        </option>
                      ))}
                    </select>
                  </td>
                  <td>
                    <AutoArea
                      className="note-area"
                      value={r.note}
                      onChange={(e) => setRule(i, { note: e.target.value })}
                      placeholder="Why, and where it applies"
                      aria-label="Details"
                    />
                  </td>
                  <td className="row-tools">
                    <button className="icon-btn" title="Move up" aria-label={`Move ${r.name || `rule ${i + 1}`} up`} onClick={() => move(i, -1)} disabled={i === 0}>
                      <ArrowUp size={14} />
                    </button>
                    <button
                      className="icon-btn"
                      title="Move down"
                      aria-label={`Move ${r.name || `rule ${i + 1}`} down`}
                      onClick={() => move(i, 1)}
                      disabled={i === draft.rules.length - 1}
                    >
                      <ArrowDown size={14} />
                    </button>
                    <button
                      className="icon-btn"
                      title="Remove rule"
                      aria-label={`Remove ${r.name || `rule ${i + 1}`}`}
                      onClick={() => setDraft((d) => ({ ...d, rules: d.rules.filter((_, k) => k !== i) }))}
                    >
                      <X size={14} />
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <div className="row">
            <button className="btn small" onClick={() => setDraft((d) => ({ ...d, rules: [...d.rules, blankRule()] }))}>
              <Plus size={13} /> Rule
            </button>
            <div className="spacer" />
            {dirty && (
              <button className="btn small primary" onClick={save} disabled={saving || !draft.name.trim()}>
                <Check size={13} /> Save rule set
              </button>
            )}
          </div>
        </section>

        <PrintLogPanel ruleSet={ruleSet} />

        <section className="panel">
          <div className="row" style={{ marginTop: 0 }}>
            <h3 className="grow" style={{ marginBottom: 0 }}>
              What agents get
            </h3>
            <button
              className="btn small"
              disabled={dirty}
              title={dirty ? 'Save the rule set first' : ''}
              onClick={() =>
                api
                  .ruleSetText(ruleSet.id)
                  .then(setPreview)
                  .catch((e) => toast.error(errText(e)))
              }
            >
              <Eye size={13} /> Show
            </button>
          </div>
          {preview && <pre className="json brief">{preview}</pre>}
        </section>

        <section className="panel danger-zone">
          {confirmDelete ? (
            <div className="row" style={{ marginTop: 0 }}>
              <button
                className="btn danger small"
                onClick={() =>
                  api
                    .deleteRuleSet(ruleSet.id)
                    .then(onDeleted)
                    .catch((e) => toast.error(errText(e)))
                }
              >
                <Trash2 size={13} /> Delete rule set
              </button>
              <button className="btn small" autoFocus onClick={() => setConfirmDelete(false)}>
                Keep
              </button>
              <span className="muted small">
                {ruleSet.always
                  ? 'It applies to every project and to agents without a project. They all lose these rules.'
                  : users.length > 0
                    ? `${users.length} project${users.length === 1 ? '' : 's'} will lose these rules.`
                    : 'No project selects it.'}
              </span>
            </div>
          ) : (
            <button ref={deleteTrigger} className="btn small ghost-danger" onClick={() => setConfirmDelete(true)}>
              <Trash2 size={13} /> Delete rule set…
            </button>
          )}
        </section>
      </div>
    </div>
  );
}
