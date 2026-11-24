import { useEffect, useMemo, useState } from "react";

/**
 * Sales request form (from input_assets/sales_request_sample.html).
 * Builds the "Field: value" summary that goes into the message box as the prompt.
 */

const BRANDS = [
  "Tourist Japan",
  "Tourist Korea",
  "Tourist Israel",
  "Tourist Italy",
  "Tourist Jordan",
  "Tourist Egypt",
  "Tourist Saudi Arabia",
  "Tourist Journey",
];
const SERVICES = ["Transfer only", "Vehicle and guide", "Guide only", "Vehicle only"];
const SEATS = ["Not required", "Child seat required", "Booster seat required", "Child seat and booster seat required"];
const TRANSFER_TYPES = ["One way", "Return"];
const DURATIONS = ["Half day (max 4.5 hours)", "Full day (max 10 hours)"];
const TICKETS = ["With entrance tickets", "Without entrance tickets"];
const MOBILITY = ["No mobility issues", "Limited mobility", "Wheelchair access required"];
const MONTH_NAMES = [
  "January", "February", "March", "April", "May", "June",
  "July", "August", "September", "October", "November", "December",
];

const range = (from: number, to: number) => Array.from({ length: to - from + 1 }, (_, i) => String(from + i));
const HOURS = Array.from({ length: 24 }, (_, h) => `${String(h).padStart(2, "0")}:00`);

function nextMonths(n = 24) {
  const now = new Date();
  return Array.from({ length: n }, (_, k) => {
    const d = new Date(now.getFullYear(), now.getMonth() + k, 1);
    return `${MONTH_NAMES[d.getMonth()]} ${d.getFullYear()}`;
  });
}

type Values = Record<FieldId, string>;
type FieldId =
  | "brand" | "booking" | "date" | "month" | "adults" | "kids" | "seat" | "service" | "transfer"
  | "duration" | "tickets" | "itinerary" | "pickup" | "dropoff" | "start" | "end" | "luggage"
  | "suitcases" | "hand" | "mobility" | "notes";

/** Field order and labels, exactly as in the summary Operations expects. */
const LABELS: [FieldId, string][] = [
  ["brand", "Brand"],
  ["booking", "Booking number or Freshdesk ticket"],
  ["date", "Date"],
  ["month", "Approximate month"],
  ["adults", "Adults"],
  ["kids", "Children"],
  ["seat", "Child seat or booster seat"],
  ["service", "Service type"],
  ["transfer", "Transfer type"],
  ["duration", "Duration"],
  ["tickets", "Entrance tickets"],
  ["itinerary", "Itinerary"],
  ["pickup", "Pick up location"],
  ["dropoff", "Drop off location"],
  ["start", "Start time"],
  ["end", "End time"],
  ["luggage", "Luggage"],
  ["suitcases", "Full size suitcases"],
  ["hand", "Hand luggage"],
  ["mobility", "Mobility level"],
  ["notes", "Specific notes"],
];
const LABEL = Object.fromEntries(LABELS) as Record<FieldId, string>;
const MULTILINE: FieldId[] = ["itinerary", "notes"];
const REQUIRED: FieldId[] = [
  "brand", "month", "booking", "adults", "kids", "service", "transfer",
  "itinerary", "pickup", "dropoff", "luggage", "mobility",
];

const EMPTY = Object.fromEntries(LABELS.map(([id]) => [id, ""])) as Values;
const DRAFT_KEY = "pricy-request-draft";

/** Which conditional fields are shown, per the sample form's rules. */
function visibility(v: Values): Record<FieldId, boolean> {
  const needsTickets = v.service === "Vehicle and guide" || v.service === "Guide only";
  const shown = Object.fromEntries(LABELS.map(([id]) => [id, true])) as Record<FieldId, boolean>;
  shown.month = !v.date;
  shown.seat = parseInt(v.kids, 10) > 0;
  shown.transfer = v.service === "Transfer only";
  shown.tickets = needsTickets;
  shown.duration = needsTickets || v.service === "Vehicle only";
  shown.suitcases = shown.hand = v.luggage === "Yes";
  return shown;
}

function isRequired(id: FieldId, v: Values, shown: Record<FieldId, boolean>) {
  if (!REQUIRED.includes(id) || !shown[id]) return false;
  if (id === "mobility") return v.service === "Vehicle and guide" || v.service === "Vehicle only";
  return true;
}

function formatDate(iso: string) {
  const [y, m, d] = iso.split("-");
  return y && m && d ? `${parseInt(d, 10)} ${MONTH_NAMES[parseInt(m, 10) - 1]} ${y}` : iso;
}

export function buildSummary(v: Values): string {
  const shown = visibility(v);
  return LABELS.filter(([id]) => shown[id] && v[id].trim())
    .map(([id, label]) => {
      const value = id === "date" ? formatDate(v[id]) : v[id].trim();
      return MULTILINE.includes(id) ? `${label}:\n${value}` : `${label}: ${value}`;
    })
    .join("\n\n");
}

function loadDraft(): Values {
  try {
    const saved = JSON.parse(localStorage.getItem(DRAFT_KEY) ?? "null");
    return saved ? { ...EMPTY, ...saved } : EMPTY;
  } catch {
    return EMPTY;
  }
}

export function RequestForm({
  onUse,
  submitLabel = "Use as request",
  compact = false,
}: {
  onUse: (summary: string) => void;
  submitLabel?: string;
  compact?: boolean;
}) {
  const [v, setV] = useState<Values>(loadDraft);
  const [invalid, setInvalid] = useState<Set<FieldId>>(new Set());
  const [error, setError] = useState("");
  const months = useMemo(() => nextMonths(), []);
  const shown = visibility(v);
  const summary = buildSummary(v);

  useEffect(() => {
    try {
      localStorage.setItem(DRAFT_KEY, JSON.stringify(v));
    } catch {
      /* drafts are a convenience only */
    }
  }, [v]);

  function set(id: FieldId, value: string) {
    setV((prev) => ({ ...prev, [id]: value }));
    setInvalid((prev) => {
      if (!prev.has(id)) return prev;
      const next = new Set(prev);
      next.delete(id);
      return next;
    });
    setError("");
  }

  function submit() {
    const missing = LABELS.map(([id]) => id).filter((id) => isRequired(id, v, shown) && !v[id].trim());
    if (missing.length) {
      setInvalid(new Set(missing));
      setError(`Please complete: ${missing.map((id) => LABEL[id]).join(", ")}.`);
      document.getElementById(`rf-${missing[0]}`)?.focus();
      return;
    }
    onUse(summary);
    clear();
  }

  function clear() {
    setV(EMPTY);
    setInvalid(new Set());
    setError("");
    try {
      localStorage.removeItem(DRAFT_KEY);
    } catch {
      /* ignore */
    }
  }

  const field = (id: FieldId, control: React.ReactNode, full = false) =>
    shown[id] && (
      <div className={full ? "rf-full" : undefined}>
        <label htmlFor={`rf-${id}`}>
          {LABEL[id]}
          {id === "month" && " (if no date)"}
          {isRequired(id, v, shown) && <span className="rf-req" aria-hidden="true">*</span>}
        </label>
        {control}
      </div>
    );

  const select = (id: FieldId, options: string[], placeholder: string) => (
    <select
      id={`rf-${id}`}
      value={v[id]}
      className={invalid.has(id) ? "invalid" : undefined}
      aria-required={isRequired(id, v, shown)}
      onChange={(e) => set(id, e.target.value)}
    >
      <option value="">{placeholder}</option>
      {options.map((o) => (
        <option key={o}>{o}</option>
      ))}
    </select>
  );

  const input = (id: FieldId, placeholder: string, type = "text") => (
    <input
      id={`rf-${id}`}
      type={type}
      value={v[id]}
      placeholder={placeholder}
      className={invalid.has(id) ? "invalid" : undefined}
      aria-required={isRequired(id, v, shown)}
      onChange={(e) => set(id, e.target.value)}
    />
  );

  const textarea = (id: FieldId, placeholder: string) => (
    <textarea
      id={`rf-${id}`}
      value={v[id]}
      placeholder={placeholder}
      rows={compact ? 3 : 4}
      className={invalid.has(id) ? "invalid" : undefined}
      aria-required={isRequired(id, v, shown)}
      onChange={(e) => set(id, e.target.value)}
    />
  );

  return (
    <div className={`request-form ${compact ? "compact" : ""}`}>
      <fieldset>
        <legend>Request</legend>
        <div className="rf-grid">
          {field("brand", select("brand", BRANDS, "Select a brand"))}
          {field("booking", input("booking", "e.g. JP85534867 or ticket number"))}
          {field("date", input("date", "", "date"))}
          {field("month", select("month", months, "Select a month"))}
          {field("adults", select("adults", range(1, 50), "Select number of adults"))}
          {field("kids", select("kids", range(0, 50), "Select number of children"))}
          {field("seat", select("seat", SEATS, "Select an option"), true)}
          {field("service", select("service", SERVICES, "Select a service type"), true)}
          {field("transfer", select("transfer", TRANSFER_TYPES, "Select a transfer type"), true)}
          {field("duration", select("duration", DURATIONS, "Select a duration"), true)}
          {field("tickets", select("tickets", TICKETS, "Select an option"), true)}
        </div>
      </fieldset>

      <fieldset>
        <legend>Itinerary</legend>
        <div className="rf-grid">
          {field("itinerary", textarea("itinerary", "Day by day overview, tours and timings"), true)}
          {field("pickup", input("pickup", "e.g. hotel name or address"))}
          {field("dropoff", input("dropoff", "e.g. hotel name or address"))}
          {field("start", select("start", HOURS, "Select a start time"))}
          {field("end", select("end", HOURS, "Select an end time"))}
          {field("luggage", select("luggage", ["Yes", "No"], "Select yes or no"), true)}
          {field("suitcases", select("suitcases", range(0, 50), "Select number of suitcases"))}
          {field("hand", select("hand", range(0, 50), "Select number of hand luggage"))}
          {field("mobility", select("mobility", MOBILITY, "Select mobility level"))}
          {field("notes", textarea("notes", "Special requests, dietary needs, timings, etc."), true)}
        </div>
      </fieldset>

      {error && (
        <p className="rf-error" role="alert">
          {error}
        </p>
      )}
      <div className="rf-actions">
        <button type="button" className="btn primary" onClick={submit}>
          {submitLabel}
        </button>
        <button type="button" className="btn" onClick={clear}>
          Clear form
        </button>
        <span className="muted small">The request is added to the message box, where you can still edit it.</span>
      </div>

      {summary && (
        <details className="rf-preview">
          <summary>Summary preview</summary>
          <pre>{summary}</pre>
        </details>
      )}
    </div>
  );
}
