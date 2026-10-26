"use client";
// [GPT-5.6] sq-lsp7k.1.2 — shared Tauri/hosted-web renderer for F1 JSON.
import * as React from "react";
import { ChevronDown, ExternalLink, Plus, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { termKey, termLabel, type FormDescription, type FormField, type FormMode, type FormValue, type TermRef } from "@/lib/form-description";
import { dashName, editorKind, emptyTermFor, normalizeBooleanLexical, viewerKind, widgetChoiceOptions } from "@/lib/form-render-model";
import { cn } from "@/lib/utils";
const CONTROL = "h-8 w-full rounded-md border bg-background px-2 text-xs outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-60";

export interface FormRendererProps {
  description: FormDescription;
  mode?: FormMode;
  onDescriptionChange?: (description: FormDescription) => void;
  onModeRequest?: (mode: FormMode) => void;
  onShapeRequest?: (shape: TermRef) => void;
  showToolbar?: boolean;
}
function safeHref(value: string): string | null {
  try { const url = new URL(value); return url.protocol === "http:" || url.protocol === "https:" ? url.href : null; } catch { return null; }
}
function replaceField(description: FormDescription, gi: number, fi: number, field: FormField): FormDescription {
  return { ...description, groups: description.groups.map((group, index) => index === gi ? { ...group, fields: group.fields.map((f, i) => i === fi ? field : f) } : group) };
}

export function FormRenderer({ description, mode, onDescriptionChange, onModeRequest, onShapeRequest, showToolbar = true }: FormRendererProps) {
  const [draft, setDraft] = React.useState(description);
  const [localMode, setLocalMode] = React.useState<FormMode>(mode ?? description.mode);
  const [previousDescription, setPreviousDescription] = React.useState(description);
  const [previousMode, setPreviousMode] = React.useState({ mode, descriptionMode: description.mode });
  if (previousDescription !== description) {
    setPreviousDescription(description);
    setDraft(description);
  }
  if (previousMode.mode !== mode || previousMode.descriptionMode !== description.mode) {
    setPreviousMode({ mode, descriptionMode: description.mode });
    setLocalMode(mode ?? description.mode);
  }
  const updateField = (gi: number, fi: number, field: FormField) => {
    const next = replaceField(draft, gi, fi, field); setDraft(next); onDescriptionChange?.(next);
  };
  return <div className="space-y-3" data-form-renderer data-form-mode={localMode}>
    {showToolbar ? <div className="flex flex-wrap items-center gap-2 rounded-md border bg-card p-2">
      <div className="min-w-0 flex-1"><p className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">Focus node</p><p className="truncate font-mono text-xs" title={draft.focus.value} data-form-focus>{draft.focus.value}</p></div>
      {draft.shapes.length > 1 ? <label className="min-w-44 text-[10px] font-medium text-muted-foreground">Shape
        <select className={cn(CONTROL, "mt-1")} value={draft.shape ? termKey(draft.shape) : ""} onChange={(event) => {
          const selected = draft.shapes.find((shape) => termKey(shape.shape) === event.currentTarget.value); if (selected) onShapeRequest?.(selected.shape);
        }} data-form-shape-switcher>{draft.shapes.map((shape) => <option key={termKey(shape.shape)} value={termKey(shape.shape)}>{shape.label ?? termLabel(shape.shape)}</option>)}</select>
      </label> : null}
      <fieldset className="flex rounded-md border p-0.5" aria-label="Form mode">{(["view", "edit"] as const).map((choice) => <button key={choice} type="button" aria-pressed={localMode === choice} onClick={() => { setLocalMode(choice); onModeRequest?.(choice); }} className={cn("rounded px-2.5 py-1 text-xs capitalize text-muted-foreground", localMode === choice && "bg-primary text-primary-foreground")} data-form-mode-choice={choice}>{choice}</button>)}</fieldset>
    </div> : null}
    {draft.groups.map((group, gi) => <details key={`${group.kind}-${group.group ? termKey(group.group) : gi}`} open={group.kind !== "other"} className="group rounded-md border bg-card" data-form-group={group.kind}>
      <summary className="flex cursor-pointer list-none items-center gap-2 px-3 py-2 text-xs font-semibold [&::-webkit-details-marker]:hidden"><ChevronDown className="size-3.5 transition-transform group-open:rotate-180" />{group.label ?? (group.kind === "default" ? "Properties" : "Other properties")}<span className="ml-auto font-normal text-muted-foreground">{group.fields.length} {group.fields.length === 1 ? "field" : "fields"}</span></summary>
      <div className="divide-y border-t">{group.fields.map((field, fi) => <FieldRow key={`${field.property_shape ? termKey(field.property_shape) : field.path}-${fi}`} field={field} mode={localMode} onChange={(next) => updateField(gi, fi, next)} />)}</div>
    </details>)}
  </div>;
}

function FieldRow({ field, mode, onChange }: { field: FormField; mode: FormMode; onChange: (field: FormField) => void }) {
  const editing = mode === "edit" && field.editable && !!field.widget.editor;
  // The widget object is preserved when only values change. Depending on the whole field here
  // would rebuild `options` and erase a user's alternative.
  const options = React.useMemo(() => widgetChoiceOptions(field.widget, editing ? "edit" : "view"), [field.widget, editing]);
  const automatic = editing ? field.widget.editor : field.widget.viewer;
  const [widget, setWidget] = React.useState(automatic ?? options[0] ?? "");
  const [previousChoice, setPreviousChoice] = React.useState({ automatic, options });
  if (previousChoice.automatic !== automatic || previousChoice.options !== options) {
    setPreviousChoice({ automatic, options });
    setWidget(automatic ?? options[0] ?? "");
  }
  const updateTerm = (index: number, term: TermRef) => { const values = field.values.slice(); values[index] = values[index] ? { ...values[index], term } : { term }; onChange({ ...field, values }); };
  const updateNested = (index: number, nested: FormDescription) => onChange({ ...field, values: field.values.map((value, i) => i === index ? { ...value, nested } : value) });
  const minimum = field.constraints.min_count ?? (field.required ? 1 : 0);
  const values = editing && field.values.length === 0 ? [{ term: emptyTermFor(field, widget) }] : field.values;
  const viewKind = viewerKind(widget);
  return <div className="grid gap-2 px-3 py-3 md:grid-cols-[minmax(9rem,0.34fr)_minmax(0,1fr)]" data-form-field={field.label}>
    <div className="min-w-0"><div className="flex items-baseline gap-1.5"><span className="text-xs font-medium">{field.label}</span>{field.required ? <span className="text-destructive" title="Required" aria-label="required">*</span> : null}{field.inverse ? <span className="rounded bg-muted px-1 text-[9px] uppercase text-muted-foreground">incoming</span> : null}</div>{field.description ? <p className="mt-0.5 text-[11px] text-muted-foreground">{field.description}</p> : null}<p className="mt-1 truncate font-mono text-[9px] text-muted-foreground" title={field.path}>{field.path}</p></div>
    <div className="min-w-0 space-y-2">
      {options.length > 1 ? <label className="flex items-center justify-end gap-2 text-[10px] text-muted-foreground">Widget<select className="h-6 max-w-48 rounded border bg-background px-1.5 text-[10px]" value={widget} onChange={(event) => setWidget(event.currentTarget.value)} data-widget-switcher>{options.map((option) => <option key={option} value={option}>{dashName(option)}</option>)}</select></label> : null}
      {editing ? <div className="space-y-2" data-field-editor={editorKind(widget)}>{values.map((value, index) => <div key={`${field.path}-${index}`} className="flex items-start gap-1.5"><div className="min-w-0 flex-1"><EditorValue field={field} value={value} widget={widget} onChange={(term) => updateTerm(index, term)} onNestedChange={(nested) => updateNested(index, nested)} /></div>{field.multi ? <Button type="button" variant="ghost" size="sm" className="size-8 shrink-0 p-0" aria-label={`Remove ${field.label} value ${index + 1}`} onClick={() => onChange({ ...field, values: field.values.filter((_, i) => i !== index) })} disabled={field.values.length <= minimum} data-remove-value><Trash2 className="size-3.5" /></Button> : null}</div>)}{field.multi ? <Button type="button" variant="outline" size="sm" onClick={() => onChange({ ...field, values: [...field.values, { term: emptyTermFor(field, widget) }] })} data-add-value><Plus className="size-3.5" />Add value</Button> : null}</div>
      : viewKind === "value-table" ? <ValueTable values={field.values} /> : field.values.length === 0 ? <p className="text-xs italic text-muted-foreground">No value</p> : <ul className="space-y-1.5" data-field-viewer={viewKind}>{field.values.map((value, index) => <li key={`${termKey(value.term)}-${index}`}><ViewerValue value={value} kind={viewKind} /></li>)}</ul>}
    </div>
  </div>;
}

function EditorValue({ field, value, widget, onChange, onNestedChange }: { field: FormField; value: FormValue; widget: string; onChange: (term: TermRef) => void; onNestedChange: (nested: FormDescription) => void }) {
  const kind = editorKind(widget); const term = value.term; const update = (next: string) => onChange({ ...term, value: next });
  if (kind === "nested" && value.nested) return <div className="rounded-md border bg-background p-2" data-nested-form><p className="mb-2 truncate font-mono text-[10px] text-muted-foreground">{term.kind === "bnode" ? `_:${term.value}` : term.value}</p><FormRenderer description={value.nested} mode="edit" showToolbar={false} onDescriptionChange={onNestedChange} /></div>;
  if (kind === "enum") { const allowed = field.constraints.in_values ?? []; return <select className={CONTROL} value={termKey(term)} onChange={(event) => { const selected = allowed.find((candidate) => termKey(candidate) === event.currentTarget.value); if (selected) onChange(selected); }} aria-label={field.label}>{allowed.map((candidate) => <option key={termKey(candidate)} value={termKey(candidate)}>{termLabel(candidate)}</option>)}</select>; }
  if (kind === "boolean") return <select className={CONTROL} value={normalizeBooleanLexical(term.value)} onChange={(event) => update(event.currentTarget.value)} aria-label={field.label}><option value="true">True</option><option value="false">False</option></select>;
  if (kind === "lang-text" || kind === "lang-textarea") return <div className="grid gap-1.5 sm:grid-cols-[minmax(0,1fr)_6rem]">{kind === "lang-textarea" ? <textarea className={cn(CONTROL, "h-20 py-1.5")} value={term.value} onChange={(event) => update(event.currentTarget.value)} aria-label={field.label} /> : <input className={CONTROL} value={term.value} onChange={(event) => update(event.currentTarget.value)} aria-label={field.label} />}<input className={CONTROL} value={term.language ?? ""} onChange={(event) => onChange({ ...term, language: event.currentTarget.value })} aria-label={`${field.label} language`} placeholder="lang" /></div>;
  if (kind === "textarea") return <textarea className={cn(CONTROL, "h-20 py-1.5")} value={term.value} onChange={(event) => update(event.currentTarget.value)} aria-label={field.label} />;
  const type = kind === "date" ? "date" : kind === "datetime" ? "datetime-local" : "text";
  return <input className={CONTROL} type={type} value={kind === "datetime" ? term.value.replace(/Z$/, "").slice(0, 16) : term.value} onChange={(event) => update(event.currentTarget.value)} aria-label={field.label} placeholder={kind === "iri-ref" ? "https://example.org/resource" : kind === "blank-node" ? "blank-node label" : undefined} spellCheck={kind !== "iri-ref"} />;
}

function ViewerValue({ value, kind }: { value: FormValue; kind: ReturnType<typeof viewerKind> }) {
  if (kind === "nested" && value.nested) return <div className="rounded-md border bg-background p-2" data-nested-form><FormRenderer description={value.nested} mode="view" showToolbar={false} /></div>;
  const term = value.term; const href = term.kind === "iri" ? safeHref(term.value) : null;
  if (kind === "hyperlink" && href) return <a href={href} target="_blank" rel="noreferrer" className="inline-flex max-w-full items-center gap-1 text-xs text-primary hover:underline"><span className="truncate">{termLabel(term)}</span><ExternalLink className="size-3" aria-hidden /></a>;
  if (kind === "image" && href) return <span className="text-xs"><a href={href} target="_blank" rel="noreferrer" className="text-primary hover:underline">{termLabel(term)}</a></span>;
  if (kind === "html") return <code className="block whitespace-pre-wrap break-words rounded bg-muted px-2 py-1 text-xs">{term.value}</code>;
  return <span className={cn("break-all text-xs", term.kind !== "literal" && "font-mono")} title={term.value}>{kind === "label" ? termLabel(term) : term.value}{term.language ? <span className="ml-1 text-[10px] text-muted-foreground">@{term.language}</span> : null}</span>;
}
function ValueTable({ values }: { values: FormValue[] }) {
  return <div className="overflow-x-auto rounded-md border" data-value-table><table className="w-full text-left text-[11px]"><thead className="bg-muted text-muted-foreground"><tr><th className="px-2 py-1">Kind</th><th className="px-2 py-1">Value</th><th className="px-2 py-1">Language / datatype</th></tr></thead><tbody className="divide-y">{values.map((value, index) => <tr key={`${termKey(value.term)}-${index}`}><td className="px-2 py-1 font-mono">{value.term.kind}</td><td className="break-all px-2 py-1">{value.term.value}</td><td className="break-all px-2 py-1 text-muted-foreground">{value.term.language ? `@${value.term.language}` : value.term.datatype ?? "—"}</td></tr>)}</tbody></table></div>;
}
