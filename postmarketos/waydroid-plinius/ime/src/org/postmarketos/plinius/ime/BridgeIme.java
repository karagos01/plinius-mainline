// SPDX-License-Identifier: GPL-3.0-or-later
package org.postmarketos.plinius.ime;

import android.content.SharedPreferences;
import android.inputmethodservice.InputMethodService;
import android.os.Handler;
import android.os.Looper;
import android.os.SystemClock;
import android.text.InputType;
import android.view.KeyEvent;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;

import java.io.BufferedReader;
import java.io.IOException;
import java.io.InputStreamReader;
import java.io.OutputStreamWriter;
import java.io.Writer;
import java.net.InetSocketAddress;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.security.SecureRandom;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/**
 * Input method without a window of its own: it asks waydroid-ime-bridge on
 * the host to raise the Phosh on-screen keyboard and applies the text that
 * comes back. See ime-bridge/src/main.rs for the protocol.
 */
public class BridgeIme extends InputMethodService {
    private static final String HOST = "192.168.240.1";
    private static final int PORT = 48613;
    private static final int RETRY_MS = 2000;
    // Android finishes and restarts the input when the window focus moves,
    // e.g. when Waydroid refocuses the app, and when Phosh shrinks the
    // window for the keyboard, apps that don't handle size changes are
    // relaunched (~0.5-0.8 s) and focus their field again. Don't drop the
    // keyboard for that.
    private static final int HIDE_DELAY_MS = 1500;

    // zwp_text_input_v3 content hints and purposes
    private static final int HINT_COMPLETION = 0x1;
    private static final int HINT_SPELLCHECK = 0x2;
    private static final int HINT_AUTO_CAPITALIZATION = 0x4;
    private static final int HINT_HIDDEN_TEXT = 0x40;
    private static final int HINT_SENSITIVE_DATA = 0x80;
    private static final int HINT_MULTILINE = 0x200;
    private static final int PURPOSE_NORMAL = 0;
    private static final int PURPOSE_DIGITS = 2;
    private static final int PURPOSE_NUMBER = 3;
    private static final int PURPOSE_PHONE = 4;
    private static final int PURPOSE_URL = 5;
    private static final int PURPOSE_EMAIL = 6;
    private static final int PURPOSE_NAME = 7;
    private static final int PURPOSE_PASSWORD = 8;
    private static final int PURPOSE_PIN = 9;
    private static final int PURPOSE_DATE = 10;
    private static final int PURPOSE_TIME = 11;
    private static final int PURPOSE_DATETIME = 12;

    private final Handler main = new Handler(Looper.getMainLooper());
    private final ExecutorService sender = Executors.newSingleThreadExecutor();
    private volatile boolean running = true;
    private volatile Socket socket;
    private volatile Writer writer;
    private String token;
    private String shown; // last SHOW line while the keyboard is wanted
    private final Runnable hide = () -> {
        if (shown != null) {
            shown = null;
            send("HIDE");
        }
    };

    @Override
    public void onCreate() {
        super.onCreate();
        token = loadToken();
        Thread t = new Thread(this::netLoop, "ime-bridge");
        t.setDaemon(true);
        t.start();
    }

    @Override
    public void onDestroy() {
        running = false;
        closeSocket();
        sender.shutdownNow();
        super.onDestroy();
    }

    private String loadToken() {
        SharedPreferences prefs = getSharedPreferences("bridge", MODE_PRIVATE);
        String t = prefs.getString("token", null);
        if (t == null) {
            byte[] b = new byte[24];
            new SecureRandom().nextBytes(b);
            StringBuilder sb = new StringBuilder();
            for (byte x : b)
                sb.append(String.format("%02x", x));
            t = sb.toString();
            prefs.edit().putString("token", t).commit();
        }
        return t;
    }

    private void netLoop() {
        while (running) {
            try (Socket s = new Socket()) {
                s.connect(new InetSocketAddress(HOST, PORT), RETRY_MS);
                s.setTcpNoDelay(true);
                BufferedReader in = new BufferedReader(
                        new InputStreamReader(s.getInputStream(), StandardCharsets.UTF_8));
                socket = s;
                writer = new OutputStreamWriter(s.getOutputStream(), StandardCharsets.UTF_8);
                send("HELLO " + token);
                main.post(() -> {
                    if (shown != null)
                        send(shown);
                });
                String line;
                while ((line = in.readLine()) != null) {
                    final String l = line;
                    main.post(() -> handle(l));
                }
            } catch (IOException e) {
                // host side not running yet; retry
            } finally {
                writer = null;
                socket = null;
            }
            SystemClock.sleep(RETRY_MS);
        }
    }

    private void closeSocket() {
        Socket s = socket;
        if (s != null) {
            try {
                s.close();
            } catch (IOException e) {
                // ignore
            }
        }
    }

    private void send(String line) {
        final Writer w = writer;
        if (w == null)
            return;
        try {
            sender.execute(() -> {
                try {
                    w.write(line);
                    w.write('\n');
                    w.flush();
                } catch (IOException e) {
                    closeSocket();
                }
            });
        } catch (java.util.concurrent.RejectedExecutionException e) {
            // shutting down
        }
    }

    private static String unescape(String s) {
        if (s.indexOf('\\') < 0)
            return s;
        StringBuilder sb = new StringBuilder(s.length());
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            if (c == '\\' && i + 1 < s.length()) {
                char n = s.charAt(++i);
                sb.append(n == 'n' ? '\n' : n == 'r' ? '\r' : n);
            } else {
                sb.append(c);
            }
        }
        return sb.toString();
    }

    /** Number of chars at the end (before) or start of s covering the UTF-8 byte count. */
    private static int charsForBytes(CharSequence cs, int bytes, boolean before) {
        if (cs == null)
            return 0;
        String s = cs.toString();
        int used = 0, i = before ? s.length() : 0;
        while (used < bytes && (before ? i > 0 : i < s.length())) {
            int cp = before ? s.codePointBefore(i) : s.codePointAt(i);
            int n = Character.charCount(cp);
            i += before ? -n : n;
            used += cp < 0x80 ? 1 : cp < 0x800 ? 2 : cp < 0x10000 ? 3 : 4;
        }
        return before ? s.length() - i : i;
    }

    private void handle(String line) {
        int sp = line.indexOf(' ');
        String cmd = sp < 0 ? line : line.substring(0, sp);
        String arg = sp < 0 ? "" : line.substring(sp + 1);
        InputConnection ic = getCurrentInputConnection();
        if (ic == null)
            return;
        switch (cmd) {
            case "COMMIT": {
                String text = unescape(arg);
                if (text.equals("\n"))
                    sendKeyChar('\n'); // runs the editor action where there is one
                else
                    ic.commitText(text, 1);
                break;
            }
            case "PREEDIT":
                ic.setComposingText(unescape(arg), 1);
                break;
            case "DELETE": {
                String[] p = arg.split(" ");
                int before = Integer.parseInt(p[0]), after = Integer.parseInt(p[1]);
                int cb = charsForBytes(ic.getTextBeforeCursor(before, 0), before, true);
                int ca = charsForBytes(ic.getTextAfterCursor(after, 0), after, false);
                ic.deleteSurroundingText(cb, ca);
                break;
            }
            case "KEY": {
                String[] p = arg.split(" ");
                int code = Integer.parseInt(p[0]);
                int meta = p.length > 1 ? Integer.parseInt(p[1]) : 0;
                if (code == KeyEvent.KEYCODE_ENTER && meta == 0) {
                    sendKeyChar('\n');
                    break;
                }
                long now = SystemClock.uptimeMillis();
                ic.sendKeyEvent(new KeyEvent(now, now, KeyEvent.ACTION_DOWN, code, 0, meta));
                ic.sendKeyEvent(new KeyEvent(now, now, KeyEvent.ACTION_UP, code, 0, meta));
                break;
            }
            default:
                break;
        }
    }

    private static String contentType(EditorInfo ei) {
        int type = ei == null ? InputType.TYPE_CLASS_TEXT : ei.inputType;
        int cls = type & InputType.TYPE_MASK_CLASS;
        int var = type & InputType.TYPE_MASK_VARIATION;
        int flags = type & InputType.TYPE_MASK_FLAGS;
        int hint = 0, purpose = PURPOSE_NORMAL;
        switch (cls) {
            case InputType.TYPE_CLASS_NUMBER:
                if (var == InputType.TYPE_NUMBER_VARIATION_PASSWORD) {
                    purpose = PURPOSE_PIN;
                    hint |= HINT_HIDDEN_TEXT | HINT_SENSITIVE_DATA;
                } else if ((flags & (InputType.TYPE_NUMBER_FLAG_DECIMAL
                        | InputType.TYPE_NUMBER_FLAG_SIGNED)) != 0) {
                    purpose = PURPOSE_NUMBER;
                } else {
                    purpose = PURPOSE_DIGITS;
                }
                break;
            case InputType.TYPE_CLASS_PHONE:
                purpose = PURPOSE_PHONE;
                break;
            case InputType.TYPE_CLASS_DATETIME:
                purpose = var == InputType.TYPE_DATETIME_VARIATION_DATE ? PURPOSE_DATE
                        : var == InputType.TYPE_DATETIME_VARIATION_TIME ? PURPOSE_TIME
                        : PURPOSE_DATETIME;
                break;
            case InputType.TYPE_CLASS_TEXT:
                switch (var) {
                    case InputType.TYPE_TEXT_VARIATION_PASSWORD:
                    case InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD:
                    case InputType.TYPE_TEXT_VARIATION_WEB_PASSWORD:
                        purpose = PURPOSE_PASSWORD;
                        hint |= HINT_SENSITIVE_DATA;
                        if (var != InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD)
                            hint |= HINT_HIDDEN_TEXT;
                        break;
                    case InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS:
                    case InputType.TYPE_TEXT_VARIATION_WEB_EMAIL_ADDRESS:
                        purpose = PURPOSE_EMAIL;
                        break;
                    case InputType.TYPE_TEXT_VARIATION_URI:
                        purpose = PURPOSE_URL;
                        break;
                    case InputType.TYPE_TEXT_VARIATION_PERSON_NAME:
                        purpose = PURPOSE_NAME;
                        break;
                    default:
                        break;
                }
                if ((flags & InputType.TYPE_TEXT_FLAG_CAP_SENTENCES) != 0)
                    hint |= HINT_AUTO_CAPITALIZATION;
                if ((flags & InputType.TYPE_TEXT_FLAG_AUTO_CORRECT) != 0)
                    hint |= HINT_SPELLCHECK;
                if ((flags & InputType.TYPE_TEXT_FLAG_AUTO_COMPLETE) != 0)
                    hint |= HINT_COMPLETION;
                if ((flags & (InputType.TYPE_TEXT_FLAG_MULTI_LINE
                        | InputType.TYPE_TEXT_FLAG_IME_MULTI_LINE)) != 0)
                    hint |= HINT_MULTILINE;
                break;
            default:
                break;
        }
        return "SHOW " + hint + " " + purpose;
    }

    @Override
    public boolean onShowInputRequested(int flags, boolean configChange) {
        main.removeCallbacks(hide);
        shown = contentType(getCurrentInputEditorInfo());
        send(shown);
        // Never show a window of our own: Waydroid would map it as a
        // separate toplevel covering the app.
        return false;
    }

    @Override
    public boolean onEvaluateInputViewShown() {
        return false;
    }

    @Override
    public boolean onEvaluateFullscreenMode() {
        return false;
    }

    @Override
    public void onStartInput(EditorInfo attribute, boolean restarting) {
        super.onStartInput(attribute, restarting);
        if (attribute == null || attribute.inputType == InputType.TYPE_NULL)
            return;
        main.removeCallbacks(hide);
        // The field changed its type while the keyboard is up.
        if (shown != null) {
            String next = contentType(attribute);
            if (!next.equals(shown)) {
                shown = next;
                send(shown);
            }
        }
    }

    private void wantHidden() {
        if (shown != null) {
            main.removeCallbacks(hide);
            main.postDelayed(hide, HIDE_DELAY_MS);
        }
    }

    @Override
    public void hideWindow() {
        wantHidden();
        super.hideWindow();
    }

    @Override
    public void onFinishInput() {
        wantHidden();
        super.onFinishInput();
    }
}
