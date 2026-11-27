import org.intralink.android.CaptureProfiles;
import org.intralink.android.CaptureRun;
import org.intralink.android.CaptureSize;

/** Tests the actual compiled pure Kotlin capture policy; no Android API calls. */
class AndroidCaptureHostCheck {
    static void check(boolean value, String label) {
        if (!value) throw new AssertionError(label);
    }

    public static void main(String[] args) {
        var profiles = CaptureProfiles.INSTANCE;
        check(new CaptureSize(1920, 1080).equals(profiles.fit(3840, 2160, 1080, 1920, 2, 2)), "landscape");
        check(new CaptureSize(864, 1920).equals(profiles.fit(1080, 2400, 1080, 1920, 16, 16)), "portrait aligned");
        check(new CaptureSize(640, 480).equals(profiles.fit(640, 480, 1080, 1920, 2, 2)), "no upscale");
        check(profiles.fit(0, 480, 1080, 1920, 2, 2) == null, "zero width");
        check(profiles.fit(640, 480, 1080, 1920, 0, 2) == null, "invalid alignment");
        check(new CaptureSize(960, 540).equals(profiles.fit(1920, 1080, 540, 960, 2, 2)), "540p fallback");

        var gate = CaptureRun.INSTANCE;
        long first = gate.reserve();
        check(first > 0 && gate.reserve() == 0, "exclusive reservation");
        check(gate.claim(first) && !gate.claim(first), "single claim");
        gate.update(new CaptureRun.Snapshot(first, CaptureRun.Phase.STARTING, "test", 1920, 1080, 60, 1, 100, 10));
        check(gate.snapshot().getPhase() == CaptureRun.Phase.RUNNING && gate.snapshot().getBuffers() == 1, "metrics");
        gate.cancel(first);
        gate.update(new CaptureRun.Snapshot(first, CaptureRun.Phase.RUNNING, "test", 1920, 1080, 60, 99, 100, 20));
        check(!gate.active(first) && gate.snapshot().getBuffers() == 1, "stop blocks stale update");
        check(gate.reserve() == 0, "no overlapping cleanup");
        gate.finish(first, CaptureRun.Phase.STOPPED, null);
        long second = gate.reserve();
        check(second > first, "unique lease");
        gate.cancel(first);
        check(gate.active(second), "stale stop isolation");
        gate.cancel(second);
        check(!gate.claim(second), "cancel before claim");
        gate.finish(second, CaptureRun.Phase.STOPPED, null);
        System.out.println("Capture policy: 6 geometry and 8 lifecycle checks PASSED (host JVM, no Android runtime).");
    }
}
