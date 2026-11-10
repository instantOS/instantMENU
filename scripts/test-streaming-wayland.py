#!/usr/bin/env python3
"""Exercise streaming interaction with real pipes and an isolated compositor.

Requires sway and wtype. Run after `cargo build --bin instantmenu`:
    python3 scripts/test-streaming-wayland.py [path/to/instantmenu]
"""

import argparse
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time


PRODUCER = """
import sys, time
from pathlib import Path
slow = sys.argv[1] == 'slow'
i = 0
if sys.argv[1].startswith('large-'):
    limit = 90000 if sys.argv[1].startswith('large-loaded') else 50000
    for start in range(0, limit, 256):
        sys.stdout.write(''.join(
            '/workspace/project-%d/src/components/directory-%d/file-%08d.rs\\n' % (n % 117, n % 53, n)
            for n in range(start, min(start + 256, limit))))
        sys.stdout.flush()
    Path(sys.argv[2]).touch()
    if limit == 90000:
        sys.exit(0)
    i = limit
while True:
    count = 1 if slow else 256
    sys.stdout.write(''.join('/workspace/file-%08d\\n' % n
                             for n in range(i, i + count)))
    sys.stdout.flush()
    i += count
    if slow:
        time.sleep(.05)
"""


def stop(process):
    if process is not None and process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


def wait_for(predicate, process, label):
    deadline = time.monotonic() + 10
    while not predicate():
        if process.poll() is not None:
            raise RuntimeError(f"{label} exited with status {process.returncode}")
        if time.monotonic() >= deadline:
            raise TimeoutError(f"{label} did not become ready within 10 seconds")
        time.sleep(.01)


def exercise(binary, env, root, label, text, key):
    producer = menu = None
    log_path = root / f"{label}.log"
    with log_path.open("w") as log:
        try:
            producer = subprocess.Popen(
                [sys.executable, "-c", PRODUCER, label, str(root / f"{label}.ready")],
                stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            )
            menu = subprocess.Popen(
                [str(binary), "--no-config", "--backend", "wayland"],
                stdin=producer.stdout, stdout=subprocess.PIPE, stderr=log,
                env={**env, "WAYLAND_DEBUG": "1"},
            )
            producer.stdout.close()
            # Wait for the actual menu frame, rather than the transparent
            # 1x1 keyboard-acquisition surface. Keep protocol traces on failure.
            wait_for(
                lambda: re.search(r"damage_buffer\(0, 0, 1280,", log_path.read_text()),
                menu, label,
            )
            if label.startswith("large-"):
                ready = root / f"{label}.ready"
                # The pipe cannot buffer this corpus: reaching the marker
                # proves the menu has consumed tens of thousands of paths.
                wait_for(ready.exists, menu, label)
            time.sleep(.3)
            if label == "slow":
                # Opening a simple menu must not allocate a search thread.
                names = [p.read_text().strip() for p in Path(f"/proc/{menu.pid}/task").glob("*/comm")]
                assert not any(name.startswith("instantmenu-sea") for name in names), names
            if not label.startswith("large-loaded"):
                assert producer.poll() is None, "producer ended before interaction"
            started = time.monotonic()
            # Give the virtual keyboard time to establish focus before pressing
            # keys. Otherwise its first press can become wl_keyboard.enter's
            # already-held key rather than a new key event.
            command = ["wtype", "-s", "200"]
            if text:
                command.append(text)
                if label.startswith("large-"):
                    # Exercise interaction during an active expensive query,
                    # rather than only canceling before the worker starts it.
                    command.extend(["-s", "200"])
            command.extend(["-k", key, "-s", "100"])
            subprocess.run(command, env=env, check=True, timeout=3)
            output, _ = menu.communicate(timeout=2)
            elapsed = time.monotonic() - started
            expected_status = 1 if key == "Escape" else 0
            assert menu.returncode == expected_status, (
                f"unexpected exit status {menu.returncode}"
            )
            if key == "Escape":
                assert output == b"", output
            else:
                assert output.startswith(b"/workspace/"), output
                assert (text or "00000000").encode() in output, output
            print(f"PASS {label}: {elapsed:.3f}s including keyboard setup")
        except Exception:
            print(log_path.read_text()[-6000:], file=sys.stderr)
            raise
        finally:
            stop(menu)
            stop(producer)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", nargs="?", default="target/debug/instantmenu")
    args = parser.parse_args()
    binary = Path(args.binary).resolve(strict=True)
    for command in ("sway", "wtype"):
        if not shutil.which(command):
            parser.error(f"{command} is required")

    with tempfile.TemporaryDirectory(prefix="instantmenu-stream-") as temporary:
        root = Path(temporary)
        config = root / "config"
        config.write_text("output * resolution 1280x720\nseat seat0 fallback true\n")
        env = {k: v for k, v in os.environ.items()
               if k not in ("WAYLAND_DISPLAY", "DISPLAY", "SWAYSOCK")}
        env.update(
            XDG_RUNTIME_DIR=str(root), WLR_BACKENDS="headless",
            WLR_RENDERER="pixman", WLR_LIBINPUT_NO_DEVICES="1",
        )
        with (root / "sway.log").open("w") as log:
            compositor = subprocess.Popen(
                ["sway", "-c", str(config)], env=env, stdout=log, stderr=log,
            )
            keyboard = None
            try:
                def sockets():
                    return [p for p in root.glob("wayland-*") if p.is_socket()]

                wait_for(sockets, compositor, "compositor")
                env["WAYLAND_DISPLAY"] = sockets()[0].name
                # A headless seat has no keyboard until a virtual one exists.
                # Keep one alive so normal early keyboard acquisition works.
                keyboard = subprocess.Popen(["wtype", "-s", "60000"], env=env)
                for label, text, key in (
                    ("slow", "", "Return"),
                    ("busy-select", "", "Return"),
                    ("busy-type", "42", "Return"),
                    ("busy-cancel", "", "Escape"),
                    ("large-stream-type", "components", "Return"),
                    ("large-stream-cancel", "components", "Escape"),
                    ("large-loaded-type", "components", "Return"),
                    ("large-loaded-cancel", "components", "Escape"),
                ):
                    exercise(binary, env, root, label, text, key)
            finally:
                stop(keyboard)
                stop(compositor)


if __name__ == "__main__":
    main()
