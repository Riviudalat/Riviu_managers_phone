package com.riviu.agent;

import android.inputmethodservice.InputMethodService;
import android.view.View;
import android.view.inputmethod.EditorInfo;

/**
 * A keyboard that does not draw one. Android 10+ lets the current IME read
 * the clipboard; the desktop driver {@code ime set}s this service for one
 * request and then restores the previous IME. Showing a panel would flash
 * over TikTok and is the mark GenFarmer leaves by staying the default IME.
 */
public final class RiviuIme extends InputMethodService {
    @Override
    public View onCreateInputView() {
        return null;
    }

    @Override
    public boolean onEvaluateInputViewShown() {
        return false;
    }

    @Override
    public void onStartInput(EditorInfo attribute, boolean restarting) {
        super.onStartInput(attribute, restarting);
        setCandidatesViewShown(false);
    }
}
