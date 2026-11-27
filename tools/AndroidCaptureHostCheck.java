import org.intralink.android.AvcObservation;
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
        gate.update(new CaptureRun.Snapshot(first, CaptureRun.Phase.STARTING, "test", 1920, 1080, 60, 1, 100, 10, new AvcObservation.Stats()));
        check(gate.snapshot().getPhase() == CaptureRun.Phase.RUNNING && gate.snapshot().getBuffers() == 1, "metrics");
        gate.cancel(first);
        gate.update(new CaptureRun.Snapshot(first, CaptureRun.Phase.RUNNING, "test", 1920, 1080, 60, 99, 100, 20, new AvcObservation.Stats()));
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
        var avc = new AvcObservation();
        avc.record((1 << 7) | (1 << 8), true, false, false, 0);
        avc.record(1 << 5, false, false, true, 10);
        avc.record(1 << 1, false, false, false, 20);
        check(!avc.canInspect(true), "partial frame not inspected");
        avc.record(-1, false, true, false, 30);
        check(!avc.canInspect(false), "continuation not inspected");
        avc.record(-1, false, false, false, 30);
        check(avc.canInspect(false), "continuation ends");
        avc.record(1 << 1, false, false, true, 20);
        avc.record(-1, false, false, false, 40);
        var stats = avc.snapshot();
        check(stats.getPictureBuffers() == 3 && stats.getIdrBuffers() == 1, "observed picture types");
        check(stats.getConfigBuffers() == 1 && stats.getParameterMask() == 384, "parameter observations");
        check(stats.getPartialBuffers() == 2 && stats.getUnparsedBuffers() == 1, "partial/invalid distinction");
        check(stats.getKeyFlagMismatches() == 1 && stats.getNonIncreasingPts() == 1, "key and timing observations");
        check(stats.getPtsSpanUs() == 10 && stats.getMinDeltaUs() == 10 && stats.getMaxDeltaUs() == 10,
              "PTS interval statistics exclude non-increasing timestamps");
        avc.record(1 << 1, false, false, false, 15);
        avc.record(1 << 1, false, false, false, 45);
        stats = avc.snapshot();
        check(stats.getNonIncreasingPts() == 2 && stats.getPtsSpanUs() == 35
              && stats.getMinDeltaUs() == 10 && stats.getMaxDeltaUs() == 25, "PTS high-water mark");
        System.out.println("Capture policy: geometry, lifecycle and AVC observation checks PASSED (host JVM, no Android runtime).");
    }
}
