package ai.storyteller.filmcraft;

import android.app.NativeActivity;
import android.content.Intent;

/**
 * The shell's activity: NativeActivity plus the one callback the NDK glue does not expose,
 * {@code onActivityResult}, which the Storage Access Framework picker needs. The native side
 * is unchanged: the inherited onCreate still dlopens libfilmcraft.so (meta-data lib_name) and
 * runs android_main.
 */
public class FilmCraftActivity extends NativeActivity {
    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        PlatformBridge.deliverResult(requestCode, resultCode, data);
    }
}
