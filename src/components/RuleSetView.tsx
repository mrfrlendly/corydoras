import { TextareaHTMLAttributes, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { api, errText, Project, Rule, RuleSet, RuleSetInput, RuleUnit } from '../api';
import { ArrowDown, ArrowUp, Check, Eye, Plus, Trash2, X } from 'lucide-react';
import { useToast } from './Toast';

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
function NumField({ value, onValue }: { value: number | null; onValue: (v: number | null) => void }) {
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

export default function RuleSetView({ ruleSet, projects, onChanged, onDeleted, onOpenProject }: Props) {
  const toast = useToast();
  const [draft, setDraft] = useState<RuleSetInput>(toInput(ruleSet));
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [preview, setPreview] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    setDraft(toInput(ruleSet));
    setPreview(null);
  }, [ruleSet]);

  const dirty = JSON.stringify(draft) !== JSON.stringify(toInput(ruleSet));
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
      await api.updateRuleSet(ruleSet.id, draft);
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
                    <input value={r.name} onChange={(e) => setRule(i, { name: e.target.value })} placeholder="Minimum wall" />
                  </td>
                  <td>
                    <NumField value={r.value} onValue={(v) => setRule(i, { value: v, unit: v === null ? null : r.unit ?? 'mm' })} />
                  </td>
                  <td>
                    <select
                      className="select small"
                      value={r.unit ?? ''}
                      disabled={r.value === null}
                      onChange={(e) => setRule(i, { unit: (e.target.value || null) as RuleUnit | null })}
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
                    />
                  </td>
                  <td className="row-tools">
                    <button className="icon-btn" title="Move up" onClick={() => move(i, -1)} disabled={i === 0}>
                      <ArrowUp size={14} />
                    </button>
                    <button className="icon-btn" title="Move down" onClick={() => move(i, 1)} disabled={i === draft.rules.length - 1}>
                      <ArrowDown size={14} />
                    </button>
                    <button
                      className="icon-btn"
                      title="Remove rule"
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
              <button className="btn small" onClick={() => setConfirmDelete(false)}>
                Keep
              </button>
              <span className="muted small">
                {users.length > 0 ? `${users.length} project${users.length === 1 ? '' : 's'} will lose these rules.` : 'No project uses it.'}
              </span>
            </div>
          ) : (
            <button className="btn small ghost-danger" onClick={() => setConfirmDelete(true)}>
              <Trash2 size={13} /> Delete rule set…
            </button>
          )}
        </section>
      </div>
    </div>
  );
}
