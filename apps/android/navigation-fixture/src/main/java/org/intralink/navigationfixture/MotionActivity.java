package org.intralink.navigationfixture;

import android.app.Activity;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.net.Uri;
import android.view.Gravity;
import android.view.WindowInsets;
import android.util.Log;
import android.media.MediaPlayer;
import android.view.WindowManager;
import android.widget.LinearLayout;
import android.widget.TextView;
import android.widget.VideoView;

/** Independent, offline test app. Plays only the generated local video. */
public final class MotionActivity extends Activity {
    private VideoView video;
    private final Handler clock = new Handler(Looper.getMainLooper());
    private final Runnable playbackSample = new Runnable() {
        @Override public void run() {
            if (video != null && video.isPlaying()) {
                Log.i("IntraLinkMotion", "PLAYBACK_POSITION " + video.getCurrentPosition());
            }
            clock.postDelayed(this, 1000);
        }
    };

    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        LinearLayout page = new LinearLayout(this);
        page.setOrientation(LinearLayout.VERTICAL);
        page.setGravity(Gravity.CENTER);
        page.setBackgroundColor(0xff18283e);
        TextView heading = new TextView(this);
        heading.setText(R.string.video_title);
        heading.setTextColor(0xffffffff);
        heading.setTextSize(22);
        heading.setGravity(Gravity.CENTER);
        page.addView(heading, new LinearLayout.LayoutParams(-1, -2));
        video = new VideoView(this);
        page.addView(video, new LinearLayout.LayoutParams(-1, 0, 1));
        page.setOnApplyWindowInsetsListener((view, insets) -> {
            android.graphics.Insets safe = insets.getInsets(WindowInsets.Type.systemBars() | WindowInsets.Type.displayCutout());
            view.setPadding(safe.left, safe.top, safe.right, safe.bottom);
            return insets;
        });
        setContentView(page);
        video.setVideoURI(Uri.parse("android.resource://" + getPackageName() + "/" + R.raw.motion));
        video.setOnPreparedListener(player -> {
            Log.i("IntraLinkMotion", "PREPARED " + player.getVideoWidth() + "x" + player.getVideoHeight() + " duration_ms=" + player.getDuration());
            player.setLooping(true);
            video.start();
        });
        video.setOnInfoListener((player, what, extra) -> {
            if (what == MediaPlayer.MEDIA_INFO_VIDEO_RENDERING_START)
                Log.i("IntraLinkMotion", "VIDEO_RENDERING_STARTED");
            return false;
        });
        video.setOnErrorListener((player, what, extra) -> {
            heading.setText(getString(R.string.video_error, what));
            return true;
        });
    }

    @Override protected void onPause() {
        clock.removeCallbacks(playbackSample);
        if (video != null) video.pause();
        super.onPause();
    }

    @Override protected void onResume() {
        super.onResume();
        if (video != null) video.start();
        clock.removeCallbacks(playbackSample);
        clock.postDelayed(playbackSample, 1000);
    }

    @Override protected void onDestroy() {
        clock.removeCallbacks(playbackSample);
        if (video != null) video.stopPlayback();
        super.onDestroy();
    }
}
