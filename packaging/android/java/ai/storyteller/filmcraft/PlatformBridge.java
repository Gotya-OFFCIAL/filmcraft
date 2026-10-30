package ai.storyteller.filmcraft;

import android.app.NativeActivity;
import android.content.ClipData;
import android.content.ContentResolver;
import android.content.Context;
import android.content.Intent;
import android.database.Cursor;
import android.media.MediaScannerConnection;
import android.net.Uri;
import android.provider.OpenableColumns;

import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;

/**
 * Platform picks and shares for the FilmCraft shell.
 *
 * Import goes through the Storage Access Framework (the system document picker): the native
 * side calls {@link #pickFiles}, the result arrives in {@code FilmCraftActivity.onActivityResult}
 * and is handed back as real paths (each content:// document is copied into the app's cache
 * first, because the engine reads ordinary files). Preview and share take a plain file path,
 * scan it into the MediaStore (which yields the content:// uri the Intent API requires on
 * API 24+) and launch ACTION_VIEW / ACTION_SEND.
 *
 * All entry points may be called from any thread; view and Intent work is posted to the UI
 * thread and the file copies run on a background thread.
 */
public final class PlatformBridge {
    /** Matches ui-egui's native_dialogs request kinds; only the request id travels. */
    public static final int REQ_PICK = 4711;

    private static NativeActivity activity;

    private static native void onPicked(String[] paths, int token);
    private static native void onPickCancelled(int token);

    private PlatformBridge() {}

    public static void install(NativeActivity a) {
        activity = a;
    }

    private static volatile int pendingToken;

    /** Open the system document picker (multi-select); answers with cache copies of the picks. */
    public static void pickFiles(final boolean multiple, final String[] mimeTypes, final int token) {
        final NativeActivity a = activity;
        if (a == null) return;
        pendingToken = token;
        a.runOnUiThread(new Runnable() {
            @Override public void run() {
                Intent i = new Intent(Intent.ACTION_OPEN_DOCUMENT);
                i.addCategory(Intent.CATEGORY_OPENABLE);
                i.setType("*/*");
                if (mimeTypes != null && mimeTypes.length > 0) {
                    i.putExtra(Intent.EXTRA_MIME_TYPES, mimeTypes);
                }
                i.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, multiple);
                try {
                    a.startActivityForResult(i, REQ_PICK);
                } catch (Exception e) {
                    android.util.Log.w("PlatformBridge", "pickFiles failed: " + e);
                    new Thread(() -> onPickCancelled(token)).start();
                }
            }
        });
    }

    /** Deliver an activity result: copy every picked document into the cache, then answer. */
    static void deliverResult(int requestCode, int resultCode, Intent data) {
        if (requestCode != REQ_PICK) return;
        // The token travels out-of-band with the request; the native side keeps one pending
        // pick at a time and the token rides along in the pending map (see platform.rs).
        final int token = pendingToken;
        if (resultCode != android.app.Activity.RESULT_OK || data == null) {
            new Thread(() -> onPickCancelled(token)).start();
            return;
        }
        final Uri[] uris = collect(data);
        new Thread(new Runnable() {
            @Override public void run() {
                String[] paths = copyAll(uris);
                if (paths.length == 0) {
                    onPickCancelled(token);
                } else {
                    onPicked(paths, token);
                }
            }
        }).start();
    }

    private static Uri[] collect(Intent data) {
        if (data.getClipData() != null) {
            ClipData clip = data.getClipData();
            Uri[] out = new Uri[clip.getItemCount()];
            for (int i = 0; i < out.length; i++) out[i] = clip.getItemAt(i).getUri();
            return out;
        }
        Uri one = data.getData();
        return one == null ? new Uri[0] : new Uri[] { one };
    }

    private static String[] copyAll(Uri[] uris) {
        NativeActivity a = activity;
        if (a == null) return new String[0];
        File dir = new File(a.getCacheDir(), "imports");
        if (!dir.isDirectory() && !dir.mkdirs()) return new String[0];
        String[] out = new String[uris.length];
        int n = 0;
        for (Uri uri : uris) {
            String dest = copyOne(a, dir, uri);
            if (dest != null) out[n++] = dest;
        }
        String[] paths = new String[n];
        System.arraycopy(out, 0, paths, 0, n);
        return paths;
    }

    private static String copyOne(Context ctx, File dir, Uri uri) {
        ContentResolver cr = ctx.getContentResolver();
        String name = displayName(cr, uri);
        File dest = unique(dir, name);
        try (InputStream in = cr.openInputStream(uri);
             OutputStream out = new FileOutputStream(dest)) {
            if (in == null) return null;
            byte[] buf = new byte[65536];
            int r;
            while ((r = in.read(buf)) > 0) out.write(buf, 0, r);
            return dest.getAbsolutePath();
        } catch (Exception e) {
            android.util.Log.w("PlatformBridge", "copy " + uri + " failed: " + e);
            return null;
        }
    }

    private static String displayName(ContentResolver cr, Uri uri) {
        try (Cursor c = cr.query(uri, new String[] { OpenableColumns.DISPLAY_NAME }, null, null, null)) {
            if (c != null && c.moveToFirst()) {
                String n = c.getString(0);
                if (n != null && !n.isEmpty()) return n;
            }
        } catch (Exception ignored) {
        }
        String last = uri.getLastPathSegment();
        return last == null ? "import.bin" : last;
    }

    private static File unique(File dir, String name) {
        File f = new File(dir, name);
        String base = name;
        String ext = "";
        int dot = name.lastIndexOf('.');
        if (dot > 0) {
            base = name.substring(0, dot);
            ext = name.substring(dot);
        }
        for (int i = 1; f.exists(); i++) f = new File(dir, base + "-" + i + ext);
        return f;
    }

    /** Open the system player on the exported file. */
    public static void preview(String path) {
        launch(path, false);
    }

    /** Open the system share sheet for the exported file. */
    public static void share(String path) {
        launch(path, true);
    }

    private static void launch(final String path, final boolean share) {
        final NativeActivity a = activity;
        if (a == null) return;
        MediaScannerConnection.scanFile(a, new String[] { path }, null,
            new MediaScannerConnection.OnScanCompletedListener() {
                @Override public void onScanCompleted(String scanned, final Uri uri) {
                    a.runOnUiThread(new Runnable() {
                        @Override public void run() {
                            try {
                                String mime = a.getContentResolver().getType(uri);
                                if (mime == null) mime = guessMime(path);
                                Intent i = new Intent(share ? Intent.ACTION_SEND : Intent.ACTION_VIEW);
                                if (share) {
                                    i.setType(mime);
                                    i.putExtra(Intent.EXTRA_STREAM, uri);
                                    i.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
                                    a.startActivity(Intent.createChooser(i, "Share"));
                                } else {
                                    i.setDataAndType(uri, mime);
                                    i.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_ACTIVITY_NEW_TASK);
                                    a.startActivity(i);
                                }
                            } catch (Exception e) {
                                android.util.Log.w("PlatformBridge", "launch failed: " + e);
                            }
                        }
                    });
                }
            });
    }

    private static String guessMime(String path) {
        String p = path.toLowerCase(java.util.Locale.ROOT);
        if (p.endsWith(".mp4") || p.endsWith(".m4v")) return "video/mp4";
        if (p.endsWith(".mov")) return "video/quicktime";
        if (p.endsWith(".webm")) return "video/webm";
        if (p.endsWith(".wav")) return "audio/wav";
        if (p.endsWith(".mp3")) return "audio/mpeg";
        if (p.endsWith(".png")) return "image/png";
        if (p.endsWith(".jpg") || p.endsWith(".jpeg")) return "image/jpeg";
        return "*/*";
    }
}
