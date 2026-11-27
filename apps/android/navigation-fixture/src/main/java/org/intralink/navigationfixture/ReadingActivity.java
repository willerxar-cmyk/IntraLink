package org.intralink.navigationfixture;

import android.app.Activity;
import android.os.Bundle;
import android.webkit.WebView;
import android.util.Log;

/** Real WebView layout and scroll; no network, accounts, JS or external content. */
public final class ReadingActivity extends Activity {
    private WebView web;

    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        setTitle(R.string.reading_title);
        web = new WebView(this);
        web.getSettings().setAllowFileAccess(false);
        web.getSettings().setAllowContentAccess(false);
        web.getSettings().setBlockNetworkLoads(true);
        web.getSettings().setJavaScriptEnabled(false);
        web.setOnScrollChangeListener((view, x, y, oldX, oldY) -> {
            if (y / 256 != oldY / 256)
                Log.i("IntraLinkReading", "SCROLL_Y " + y);
        });
        setContentView(web);
        StringBuilder html = new StringBuilder("<!doctype html><html><head><meta name='viewport' content='width=device-width,initial-scale=1'><style>body{font-family:sans-serif;margin:0;background:#eef2f6;color:#19283b}header{position:sticky;top:0;background:#1c3048;color:white;padding:22px}main{padding:18px}section{background:white;border-radius:14px;padding:18px;margin:12px 0}p{line-height:1.6}article{height:150px;background:linear-gradient(125deg,#128a99,#a6d5cb,#365779);border-radius:10px}</style></head><body><header>INTRALINK | Offline reading and scrolling</header><main>");
        for (int i = 1; i <= 40; i++) {
            html.append("<section><h2>Rendering sample ").append(i).append("</h2><article></article><p>Local text, gradient panels, changing scroll position and native browser compositing. This document contains only generated test content.</p></section>");
        }
        html.append("</main></body></html>");
        web.loadDataWithBaseURL(null, html.toString(), "text/html", "UTF-8", null);
    }

    @Override protected void onDestroy() {
        web.destroy();
        super.onDestroy();
    }
}
