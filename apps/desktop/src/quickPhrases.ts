/**
 * Saved snippets the operator types onto the selected phones in one click.
 *
 * GenFarmer calls this "Quick phase", which is a mistranslation its own UI gives away: the
 * modal is an input placeheld `Content (e.g: hello world)`, an Add button, and a list whose
 * empty state reads `No phases.` — clicking an entry sends `{text, udids}` and reports
 * "Quick phase completed!". It is a **phrase** book, not a workflow phase, and it exists
 * because typing the same bio or comment onto twenty phones by hand is the thing a farm
 * console is for.
 *
 * Kept in `localStorage` rather than the database on purpose: it is per-operator text with
 * no device in it, the same class of preference as the tile width, and putting it in SQLite
 * would mean a migration and a command surface for something that never leaves this window.
 *
 * The text is sent through the existing `group_input` `type` path, which goes to the agent's
 * `ACTION_SET_TEXT` — the one that carries full Unicode. `adb shell input text` cannot type
 * Vietnamese with diacritics at all (AGENTS.md 9.71), and this is exactly the feature that
 * would be used for it.
 */

export interface QuickPhrase {
  id: string;
  name: string;
  content: string;
}

const KEY = "riviu.quickPhrases";

/// Enough to be a phrase book, few enough that the list stays a list. A cap at all because
/// this is unbounded operator input going into a storage that has no quota errors worth
/// showing.
export const MAX_QUICK_PHRASES = 50;
/// Long enough for a bio or a comment; short enough that one bad paste cannot fill the store.
export const MAX_QUICK_PHRASE_LENGTH = 500;

/// Tolerant of anything already in storage: a shape we do not recognise is dropped rather
/// than thrown, because a corrupt preference must not stop the overlay from opening.
export function parseQuickPhrases(raw: string | null): QuickPhrase[] {
  if (!raw) return [];
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed
      .filter(
        (entry): entry is QuickPhrase =>
          typeof entry === "object" &&
          entry !== null &&
          typeof (entry as QuickPhrase).id === "string" &&
          typeof (entry as QuickPhrase).name === "string" &&
          typeof (entry as QuickPhrase).content === "string",
      )
      .slice(0, MAX_QUICK_PHRASES);
  } catch {
    return [];
  }
}

export function loadQuickPhrases(): QuickPhrase[] {
  try {
    return parseQuickPhrases(localStorage.getItem(KEY));
  } catch {
    return [];
  }
}

export function storeQuickPhrases(phrases: QuickPhrase[]): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(phrases.slice(0, MAX_QUICK_PHRASES)));
  } catch {
    // A full or disabled storage must not lose the gesture the operator is in the middle of.
  }
}

/// Add one phrase, or refuse with a reason.
///
/// Returns the new list and a message when something was rejected, rather than throwing:
/// every caller here is a click handler, and a thrown error in one is a blank panel.
export function addQuickPhrase(
  phrases: QuickPhrase[],
  name: string,
  content: string,
  id: string,
): { phrases: QuickPhrase[]; error: string | null } {
  const trimmedContent = content.trim();
  if (!trimmedContent) {
    return { phrases, error: "Chưa có nội dung để lưu." };
  }
  if (trimmedContent.length > MAX_QUICK_PHRASE_LENGTH) {
    return {
      phrases,
      error: `Nội dung dài quá ${MAX_QUICK_PHRASE_LENGTH} ký tự.`,
    };
  }
  if (phrases.length >= MAX_QUICK_PHRASES) {
    return { phrases, error: `Đã đủ ${MAX_QUICK_PHRASES} câu, xoá bớt rồi thêm.` };
  }
  // The name is a label for the list; falling back to the content keeps a nameless entry
  // findable instead of rendering an empty row.
  const trimmedName = name.trim() || trimmedContent.slice(0, 40);
  return {
    phrases: [...phrases, { id, name: trimmedName, content: trimmedContent }],
    error: null,
  };
}

export function removeQuickPhrase(phrases: QuickPhrase[], id: string): QuickPhrase[] {
  return phrases.filter((phrase) => phrase.id !== id);
}
