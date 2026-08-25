import { useState } from "react";

import {
  addQuickPhrase,
  loadQuickPhrases,
  removeQuickPhrase,
  storeQuickPhrases,
  type QuickPhrase,
} from "../../quickPhrases";

/**
 * The saved snippets the focus overlay can type into a phone.
 *
 * Lifted out of `FocusStream`, which was 1,269 lines and twelve `useState`. This cluster is
 * four of them and touches no device: the list lives in browser storage, and `quickPhrases.ts`
 * — already pure and already tested — owns the rules about what a valid one is. What was
 * stuck in the component was the form around it, and that is the part nothing could reach.
 */
export interface QuickPhrasesForm {
  phrases: QuickPhrase[];
  name: string;
  setName: (value: string) => void;
  content: string;
  setContent: (value: string) => void;
  /// Why the last save was refused, or null.
  error: string | null;
  /// Add the phrase currently in the form. Clears the form only when it was accepted.
  save: () => void;
  remove: (id: string) => void;
}

export function useQuickPhrases(): QuickPhrasesForm {
  const [phrases, setPhrases] = useState<QuickPhrase[]>(() => loadQuickPhrases());
  const [name, setName] = useState("");
  const [content, setContent] = useState("");
  const [error, setError] = useState<string | null>(null);

  const save = () => {
    const { phrases: next, error: refusal } = addQuickPhrase(
      phrases,
      name,
      content,
      `${Date.now()}-${phrases.length}`,
    );
    setError(refusal);
    if (refusal) return;
    setPhrases(next);
    storeQuickPhrases(next);
    setName("");
    setContent("");
  };

  const remove = (id: string) => {
    const next = removeQuickPhrase(phrases, id);
    setPhrases(next);
    storeQuickPhrases(next);
  };

  return { phrases, name, setName, content, setContent, error, save, remove };
}
