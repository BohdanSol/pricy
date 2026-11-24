//! Request workflow from architecture/expected_workflow.png as a finite state machine.
//! Each chat is one request; its `status` only changes through `next()`.
//! Diagrams and the UI action for every transition: architecture/WORKFLOW.md.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// No answer yet.
    New,
    /// Step 2 found a price (branch A).
    Answered,
    /// Step 2 found nothing, or the answer was marked not correct (branch B).
    NotFound,
    /// A3: sent to Operations for approval.
    PendingApproval,
    /// A3 done: Operations approved; Claude writes the final answer next.
    Approved,
    /// B3: Claude drafted a message to the supplier.
    SupplierDraft,
    /// B4: the draft was sent to Operations to forward to the supplier.
    WaitingSupplier,
    /// B5: the supplier's answer was recorded and saved for future requests.
    SupplierAnswered,
    /// A4 / B6: Claude wrote the final answer for Sales.
    FinalReady,
    /// A4 / B6 done: the final answer was sent to Sales.
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// A search turn ended with `[[pricy:found]]`.
    Found,
    /// A search turn ended with `[[pricy:not_found]]`.
    NotFound,
    SentForApproval,
    Approved,
    /// The answer is not correct (by Ops, or by Sales before asking Ops).
    Rejected,
    SupplierDrafted,
    SentToOps,
    SupplierAnswered,
    FinalReady,
    SentToSales,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::New => "new",
            Status::Answered => "answered",
            Status::NotFound => "not_found",
            Status::PendingApproval => "pending_approval",
            Status::Approved => "approved",
            Status::SupplierDraft => "supplier_draft",
            Status::WaitingSupplier => "waiting_supplier",
            Status::SupplierAnswered => "supplier_answered",
            Status::FinalReady => "final_ready",
            Status::Completed => "completed",
        }
    }

    pub fn parse(s: &str) -> Status {
        match s {
            "answered" => Status::Answered,
            "not_found" => Status::NotFound,
            "pending_approval" => Status::PendingApproval,
            "approved" => Status::Approved,
            "supplier_draft" => Status::SupplierDraft,
            "waiting_supplier" => Status::WaitingSupplier,
            "supplier_answered" => Status::SupplierAnswered,
            "final_ready" => Status::FinalReady,
            "completed" => Status::Completed,
            _ => Status::New,
        }
    }
}

impl Event {
    pub fn parse(s: &str) -> Option<Event> {
        Some(match s {
            "sent_for_approval" => Event::SentForApproval,
            "approved" => Event::Approved,
            "rejected" => Event::Rejected,
            "sent_to_ops" => Event::SentToOps,
            "sent_to_sales" => Event::SentToSales,
            _ => return None,
        })
    }
}

/// The transition function. `None` = the event is not allowed in this state.
pub fn next(s: Status, e: Event) -> Option<Status> {
    use Event as E;
    use Status as S;
    Some(match (s, e) {
        // Step 2: a search (or a follow-up search) sets the branch, until the request moves on.
        (S::New | S::Answered | S::NotFound, E::Found) => S::Answered,
        (S::New | S::Answered | S::NotFound, E::NotFound) => S::NotFound,

        // Branch A: approval by Operations.
        (S::Answered | S::PendingApproval, E::SentForApproval) => S::PendingApproval,
        (S::PendingApproval, E::Approved) => S::Approved,
        (S::Answered | S::PendingApproval, E::Rejected) => S::NotFound,
        (S::Approved, E::FinalReady) => S::FinalReady,

        // Branch B: ask the supplier through Operations.
        (S::Answered | S::NotFound | S::SupplierDraft, E::SupplierDrafted) => S::SupplierDraft,
        (S::SupplierDraft | S::WaitingSupplier, E::SentToOps) => S::WaitingSupplier,
        (S::SupplierDraft | S::WaitingSupplier, E::SupplierAnswered) => S::SupplierAnswered,
        (S::SupplierAnswered, E::FinalReady) => S::FinalReady,

        // End: the final answer goes to Sales (can be re-sent).
        (S::FinalReady | S::Completed, E::SentToSales) => S::Completed,

        _ => return None,
    })
}

// ---------- search outcome marker ----------

pub const MARKER_FOUND: &str = "[[pricy:found]]";
pub const MARKER_NOT_FOUND: &str = "[[pricy:not_found]]";

/// Removes the outcome marker Claude adds to search answers, returning the outcome.
pub fn strip_outcome(text: &str) -> (String, Option<Event>) {
    let mut outcome = None;
    let kept: Vec<&str> = text
        .lines()
        .filter(|line| {
            let t = line.trim();
            if t.contains(MARKER_FOUND) {
                outcome = Some(Event::Found);
                t != MARKER_FOUND
            } else if t.contains(MARKER_NOT_FOUND) {
                outcome = Some(Event::NotFound);
                t != MARKER_NOT_FOUND
            } else {
                true
            }
        })
        .collect();
    let cleaned = kept
        .join("\n")
        .replace(MARKER_FOUND, "")
        .replace(MARKER_NOT_FOUND, "");
    (cleaned.trim().to_string(), outcome)
}

// ---------- steps Claude performs ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// B3: draft the supplier message.
    DraftSupplier,
    /// A4: final answer after approval.
    FinalFromApproval,
    /// B6: final answer from the supplier's reply.
    FinalFromSupplier,
}

impl Action {
    /// Shown in the chat as the step that was run.
    pub fn label(self) -> &'static str {
        match self {
            Action::DraftSupplier => "Draft a message to the supplier",
            Action::FinalFromApproval => {
                "Write the Supplier Notes for Sales (approved by Operations)"
            }
            Action::FinalFromSupplier => {
                "Write the Supplier Notes for Sales from the supplier's reply"
            }
        }
    }

    /// Kind of the assistant message the step produces.
    pub fn output_kind(self) -> &'static str {
        match self {
            Action::DraftSupplier => "supplier_draft",
            _ => "final",
        }
    }

    /// Event fired when the step finishes successfully.
    pub fn done_event(self) -> Event {
        match self {
            Action::DraftSupplier => Event::SupplierDrafted,
            _ => Event::FinalReady,
        }
    }

    /// States the step may start from.
    pub fn allowed_from(self, s: Status) -> bool {
        match self {
            Action::DraftSupplier => next(s, Event::SupplierDrafted).is_some(),
            Action::FinalFromApproval => s == Status::Approved || s == Status::FinalReady,
            Action::FinalFromSupplier => s == Status::SupplierAnswered || s == Status::FinalReady,
        }
    }

    pub fn prompt(self, ctx: &StepContext) -> String {
        match self {
            Action::DraftSupplier => {
                format!("{SUPPLIER_MESSAGE_INSTRUCTIONS}\n\n{SUPPLIER_MESSAGE_TEMPLATE}")
            }
            Action::FinalFromApproval => format!(
                "Operations approved the price you found above.{}\n\n{}",
                ctx.note
                    .as_deref()
                    .filter(|n| !n.trim().is_empty())
                    .map(|n| format!(
                        " Their choice / note: \"{}\". Use the option they chose.",
                        n.trim()
                    ))
                    .unwrap_or_default(),
                supplier_notes(ctx)
            ),
            Action::FinalFromSupplier => format!(
                "The supplier answered (recorded on {}):\n\n\"\"\"\n{}\n\"\"\"\n\nUse this supplier reply as the price.\n\n{}",
                today(),
                ctx.supplier_answer.as_deref().unwrap_or("").trim(),
                supplier_notes(ctx)
            ),
        }
    }
}

/// What a workflow step needs to know beyond the conversation.
pub struct StepContext {
    /// Sales representative who requested the price (Settings → Your name).
    pub requester: Option<String>,
    /// Operations representative who approved / forwarded the price.
    pub approver: Option<String>,
    /// Operations' choice or note when approving.
    pub note: Option<String>,
    pub supplier_answer: Option<String>,
    /// Stands in for "Claude Output Link": Pricy requests are local, so there is no URL.
    pub request_ref: String,
}

const SUPPLIER_MESSAGE_INSTRUCTIONS: &str = "Fill in the message Operations will send to the supplier to request a quotation for this request. Use exactly the template below: keep its wording and line order, and only fill in the fields. Rules:
- Fill every field from the request and the conversation.
- Number of Pax: adults + children, with the split if there are children (e.g. \"3 (2 adults, 1 child, booster seat required)\").
- Luggage / No Luggage: \"No Luggage\", or the counts (e.g. \"Luggage: 2 full size suitcases, 2 hand luggage\"); \"To be confirmed\" if unknown.
- Service Type: choose one of Tours / Transfers / Round-trip / Others (Transfer only + One way = Transfers; Transfer only + Return = Round-trip; Vehicle and guide or Guide only = Tours; Vehicle only = Others, with the duration). Add the duration, entrance tickets and start / end times next to it when known.
- Mention a mobility need or child seat in the Itinerary line or after Drop-off only if the request states it.
- Itinerary: fill it for tours or multi-stop services, otherwise write \"N/A\".
- Write \"To be confirmed\" for any other detail that is not known. Never invent details.
- Output only the filled message, with no commentary and no outcome marker.";

const SUPPLIER_MESSAGE_TEMPLATE: &str = "Dear Team,
Hope you are doing well.
We would like to request a quotation for the following service.
Please see the details below:

Date:
Number of Pax:
Luggage / No Luggage:
Service Type: Tours / Transfers / Round-trip / Others
Itinerary (if relevant):
Pick-up:
Drop-off:

We would appreciate it if you could confirm whether you can accommodate this request and provide us with a price for this service.
We look forward to your response and thank you in advance for your assistance.
Best regards,";

fn supplier_notes(ctx: &StepContext) -> String {
    let requester = ctx
        .requester
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("Not stated");
    let approver = ctx
        .approver
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("Not stated");
    format!(
        "Now write the Supplier Notes for the Sales representative, who copies them into the Supplier Notes section in TINT. Use exactly this format, one field per line, with the field names in bold:

**Service Name & Description:** one sentence describing the service (date, pax, route or tour).
**Price Requested By:** {requester}
**Price Approved By:** {approver}
**Net Price:** the approved price with currency and unit, followed by \"(Net Price)\".
**Supplier Name:** the supplier of that price.
**Are there additional quotes from other suppliers?** Yes or No. If Yes, list every other option as \"Supplier – Net Price\".
**Claude Output Link:** {}

Only use facts from the conversation; write \"Not stated\" for anything unknown. Output only the Supplier Notes, with no commentary and no outcome marker.",
        ctx.request_ref
    )
}

// ---------- saved answers (B5) ----------

pub fn answers_dir(workspace: &Path) -> PathBuf {
    workspace.join("sources").join("answers")
}

/// Saves a supplier answer as a Markdown file in the "Saved answers" source,
/// so future searches find it. Returns the file name.
pub fn save_answer(
    workspace: &Path,
    chat_id: &str,
    title: &str,
    request: &str,
    answer: &str,
) -> std::io::Result<String> {
    let dir = answers_dir(workspace);
    std::fs::create_dir_all(&dir)?;
    let date = today();
    let name = format!("{date}-{}.md", &chat_id[..8.min(chat_id.len())]);
    let body = format!(
        "# {title}\n\n- Recorded: {date}\n- Source: supplier reply, forwarded by Operations\n\n## Request\n\n{}\n\n## Supplier answer\n\n{}\n",
        request.trim(),
        answer.trim()
    );
    std::fs::write(dir.join(&name), body)?;
    Ok(name)
}

/// Today's date as YYYY-MM-DD (UTC), without a date library.
pub fn today() -> String {
    date_of(crate::db::now_ms())
}

/// A millisecond timestamp as YYYY-MM-DD (UTC).
pub fn date_of(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use Event as E;
    use Status as S;

    #[test]
    fn branch_a_happy_path() {
        let mut s = S::New;
        for e in [
            E::Found,
            E::SentForApproval,
            E::Approved,
            E::FinalReady,
            E::SentToSales,
        ] {
            s = next(s, e).unwrap_or_else(|| panic!("{e:?} not allowed from {s:?}"));
        }
        assert_eq!(s, S::Completed);
    }

    #[test]
    fn branch_b_happy_path_and_rejection() {
        let mut s = S::New;
        for e in [
            E::Found,
            E::SentForApproval,
            E::Rejected,
            E::SupplierDrafted,
            E::SentToOps,
            E::SupplierAnswered,
            E::FinalReady,
            E::SentToSales,
        ] {
            s = next(s, e).unwrap_or_else(|| panic!("{e:?} not allowed from {s:?}"));
        }
        assert_eq!(s, S::Completed);
    }

    #[test]
    fn invalid_transitions_are_rejected() {
        assert_eq!(next(S::New, E::Approved), None);
        assert_eq!(next(S::NotFound, E::SentForApproval), None);
        assert_eq!(next(S::PendingApproval, E::Found), None);
        assert_eq!(next(S::Completed, E::Found), None);
    }

    #[test]
    fn strips_marker() {
        let (t, o) = strip_outcome("€620, 4h40\n\n[[pricy:found]]");
        assert_eq!(t, "€620, 4h40");
        assert_eq!(o, Some(E::Found));
        let (t, o) = strip_outcome("Nothing found. [[pricy:not_found]]");
        assert_eq!(t, "Nothing found.");
        assert_eq!(o, Some(E::NotFound));
    }

    #[test]
    fn today_is_iso_date() {
        let d = today();
        assert_eq!(d.len(), 10);
        assert!(d.starts_with("20"));
    }
}

#[cfg(test)]
mod prompt_tests {
    use super::*;

    fn ctx() -> StepContext {
        StepContext {
            requester: Some("Anna".into()),
            approver: Some("Marco".into()),
            note: Some("Option 2".into()),
            supplier_answer: Some("€640 net".into()),
            request_ref: "Pricy request \"X\"".into(),
        }
    }

    #[test]
    fn supplier_message_uses_template() {
        let p = Action::DraftSupplier.prompt(&ctx());
        assert!(p.contains("Dear Team,\nHope you are doing well."));
        assert!(p.contains("Luggage / No Luggage:"));
    }

    #[test]
    fn supplier_notes_carry_names_and_choice() {
        let p = Action::FinalFromApproval.prompt(&ctx());
        assert!(p.contains("**Price Requested By:** Anna"));
        assert!(p.contains("**Price Approved By:** Marco"));
        assert!(p.contains("Option 2"));
        let p = Action::FinalFromSupplier.prompt(&ctx());
        assert!(p.contains("€640 net"));
        assert!(p.contains("**Claude Output Link:** Pricy request"));
    }
}
