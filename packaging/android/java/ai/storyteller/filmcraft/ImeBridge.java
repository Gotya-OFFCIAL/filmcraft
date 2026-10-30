package ai.storyteller.filmcraft;

import android.app.NativeActivity;
import android.content.Context;
import android.text.Editable;
import android.text.TextWatcher;
import android.view.KeyEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputMethodManager;
import android.widget.EditText;
import android.widget.TextView;

/**
 * Hidden EditText that owns the IME's InputConnection for the FilmCraft shell.
 *
 * NativeActivity's content view is a SurfaceView: InputMethodManager treats it as a
 * non-editor, so showSoftInput() silently does nothing and typed text has nowhere to
 * go (winit's Android back end never receives Ime events). This bridge keeps a 1x1
 * transparent EditText in the view tree instead. It is a real text editor, so the IME
 * attaches, and every edit is forwarded to the native side:
 *
 *   onTextChanged(text, selStart, selEnd)  full-state after each edit
 *   onAction(actionId)                     the IME action key (Done, Next, ...)
 *
 * The native side mirrors text/selection back through update() so the EditText and
 * the application buffer stay the same document. All view work runs on the UI thread;
 * the static entry points may be called from any thread and post internally.
 */
public final class ImeBridge {
    private static EditText edit;
    private static volatile String last = "";
    private static volatile boolean echo;

    private static native void onTextChanged(String text, int selStart, int selEnd);
    private static native void onAction(int action);

    private ImeBridge() {}

    /** Attach the hidden editor to the activity's view tree (idempotent). */
    public static void install(final NativeActivity activity) {
        activity.runOnUiThread(new Runnable() {
            @Override public void run() {
                if (edit != null) return;
                EditText e = new EditText(activity);
                e.setSingleLine(false);
                // No IME_FLAG_NO_EXTRACT_UI: in landscape an IME may only offer its extract
                // (fullscreen) form, and suppressing that makes some keyboards show nothing
                // at all (observed with Sogou on MuMu: window "shown" at zero height).
                e.setImeOptions(EditorInfo.IME_ACTION_DONE);
                e.setInputType(android.text.InputType.TYPE_CLASS_TEXT | android.text.InputType.TYPE_TEXT_FLAG_MULTI_LINE);
                e.setFocusable(true);
                e.setFocusableInTouchMode(true);
                e.setAlpha(0.0f);
                e.setCursorVisible(false);
                activity.addContentView(e, new ViewGroup.LayoutParams(1, 1));
                e.addTextChangedListener(new TextWatcher() {
                    @Override public void beforeTextChanged(CharSequence s, int a, int b, int c) {}
                    @Override public void onTextChanged(CharSequence s, int a, int b, int c) {}
                    @Override public void afterTextChanged(Editable s) {
                        if (echo) return;
                        String t = s.toString();
                        if (t.equals(last)) return;
                        last = t;
                        ImeBridge.onTextChanged(t, e.getSelectionStart(), e.getSelectionEnd());
                    }
                });
                e.setOnEditorActionListener(new EditText.OnEditorActionListener() {
                    @Override public boolean onEditorAction(TextView v, int actionId, KeyEvent event) {
                        ImeBridge.onAction(actionId);
                        return true;
                    }
                });
                edit = e;
                // Self-test of the Java -> Rust hop (no IME required): the bridge must be
                // able to call its natives, which the log side records as "action -99".
                ImeBridge.onAction(-99);
            }
        });
    }

    /** Give the IME `text`/selection and show the keyboard for the hidden editor. */
    public static void show(final String text, final int selStart, final int selEnd) {
        set(text, selStart, selEnd, true);
    }

    /** Mirror application-side edits into the editor without echoing them back. */
    public static void update(String text, int selStart, int selEnd) {
        set(text, selStart, selEnd, false);
    }

    /** Hide the keyboard and drop focus. */
    public static void hide() {
        final EditText e = edit;
        if (e == null) return;
        e.post(new Runnable() {
            @Override public void run() {
                InputMethodManager imm = (InputMethodManager) e.getContext()
                    .getSystemService(Context.INPUT_METHOD_SERVICE);
                if (imm != null) imm.hideSoftInputFromWindow(e.getWindowToken(), 0);
                e.clearFocus();
            }
        });
    }

    private static void set(final String text, final int selStart, final int selEnd, final boolean showIme) {
        final EditText e = edit;
        if (e == null) return;
        e.post(new Runnable() {
            @Override public void run() {
                echo = true;
                int n = text.length();
                int a = Math.max(0, Math.min(selStart, n));
                int b = Math.max(a, Math.min(selEnd, n));
                e.setText(text);
                e.setSelection(a, b);
                echo = false;
                last = text;
                if (showIme) {
                    // requestFocus alone can lose to the activity's SurfaceView; fromTouch
                    // is the strong form. restartInput rebinds the InputConnection after the
                    // focus change, and toggleSoftInput is the last-resort kick for IMEs that
                    // ignore showSoftInput (observed: window "shown" at zero height).
                    boolean focused = e.requestFocusFromTouch();
                    InputMethodManager imm = (InputMethodManager) e.getContext()
                        .getSystemService(Context.INPUT_METHOD_SERVICE);
                    if (imm != null) {
                        imm.restartInput(e);
                        boolean shown = imm.showSoftInput(e, 0);
                        if (!shown) imm.toggleSoftInput(0, 0);
                        android.util.Log.i("ImeBridge",
                            "show: focused=" + focused + " shown=" + shown
                                + " textLen=" + text.length());
                    }
                }
            }
        });
    }
}
