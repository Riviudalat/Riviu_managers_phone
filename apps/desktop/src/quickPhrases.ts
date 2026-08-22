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
  /// Optional group label (xiaowei `quickReply` organises phrases into groups). Absent =
  /// the default group; kept optional so every phrase saved before groups existed still loads.
  group?: string;
}

const KEY = "riviu.quickPhrases";

/// The bucket an ungrouped phrase belongs to. A real name, not "" , so the picker shows it.
export const DEFAULT_QUICK_GROUP = "Chung";

/// Empty/whitespace group folds to the default, so a blank input never creates a nameless
/// group nobody can select back out of.
export function normalizeGroup(group: string | undefined): string {
  const g = (group ?? "").trim();
  return g.length ? g : DEFAULT_QUICK_GROUP;
}

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
      // Normalise the group here so the rest of the app never has to handle both a missing
      // and an empty group.
      .map((entry) => ({ ...entry, group: normalizeGroup(entry.group) }))
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
  group?: string,
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
    phrases: [
      ...phrases,
      { id, name: trimmedName, content: trimmedContent, group: normalizeGroup(group) },
    ],
    error: null,
  };
}

export function removeQuickPhrase(phrases: QuickPhrase[], id: string): QuickPhrase[] {
  return phrases.filter((phrase) => phrase.id !== id);
}

/// Distinct group names, default first, the rest in first-seen order — a stable order for the
/// group picker that does not jump around as phrases are added.
export function groupsOf(phrases: QuickPhrase[]): string[] {
  const seen = new Set<string>();
  const order: string[] = [];
  for (const phrase of phrases) {
    const g = normalizeGroup(phrase.group);
    if (!seen.has(g)) {
      seen.add(g);
      order.push(g);
    }
  }
  order.sort((a, b) => {
    if (a === DEFAULT_QUICK_GROUP) return -1;
    if (b === DEFAULT_QUICK_GROUP) return 1;
    return 0;
  });
  return order;
}

export function phrasesInGroup(phrases: QuickPhrase[], group: string): QuickPhrase[] {
  const g = normalizeGroup(group);
  return phrases.filter((phrase) => normalizeGroup(phrase.group) === g);
}

/// Export as tab-separated `group<TAB>name<TAB>content`, one phrase per line. Tabs are chosen
/// over the `|`/comma a phrase might contain; newlines in content are escaped to `\n` so one
/// phrase stays one line and the file round-trips through {@link importQuickPhrases}.
export function exportQuickPhrases(phrases: QuickPhrase[]): string {
  return phrases
    .map((p) => {
      const esc = (s: string) => s.replace(/\\/g, "\\\\").replace(/\t/g, " ").replace(/\r?\n/g, "\\n");
      return `${esc(normalizeGroup(p.group))}\t${esc(p.name)}\t${esc(p.content)}`;
    })
    .join("\n");
}

function unescape(s: string): string {
  return s.replace(/\\n/g, "\n").replace(/\\\\/g, "\\");
}

/// Import a file produced by {@link exportQuickPhrases}, tolerant of hand-written files:
/// a line may be `group<TAB>name<TAB>content`, `group<TAB>content` (name derived), or just
/// `content` (default group). Blank lines are skipped; the total is capped at
/// `MAX_QUICK_PHRASES`. Returns the merged list plus counts so the UI can report what landed.
export function importQuickPhrases(
  existing: QuickPhrase[],
  raw: string,
  idGen: () => string,
): { phrases: QuickPhrase[]; added: number; skipped: number } {
  let phrases = [...existing];
  let added = 0;
  let skipped = 0;
  for (const line of raw.split(/\r?\n/)) {
    if (!line.trim()) continue;
    const cols = line.split("\t");
    let group: string;
    let name: string;
    let content: string;
    if (cols.length >= 3) {
      group = unescape(cols[0]);
      name = unescape(cols[1]);
      content = unescape(cols.slice(2).join("\t"));
    } else if (cols.length === 2) {
      group = unescape(cols[0]);
      content = unescape(cols[1]);
      name = "";
    } else {
      group = DEFAULT_QUICK_GROUP;
      content = unescape(cols[0]);
      name = "";
    }
    const result = addQuickPhrase(phrases, name, content, idGen(), group);
    if (result.error) {
      skipped += 1;
    } else {
      phrases = result.phrases;
      added += 1;
    }
  }
  return { phrases, added, skipped };
}
