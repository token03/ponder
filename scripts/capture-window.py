import argparse
import ctypes
import ctypes.util
import os
from pathlib import Path
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description="Capture a running ponder X11 window, including under WSLg.")
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--size", nargs=2, type=int, metavar=("WIDTH", "HEIGHT"))
    interaction = parser.add_mutually_exclusive_group()
    interaction.add_argument("--click", nargs=2, type=int, metavar=("X", "Y"))
    interaction.add_argument("--scroll", nargs=4, type=int, metavar=("X", "Y", "RIGHT", "DOWN"))
    parser.add_argument("--delay", type=float, default=2.0)
    args = parser.parse_args()
    if args.delay < 0:
        parser.error("delay must be nonnegative")

    display_name = os.environ.get("DISPLAY")
    if not display_name and Path("/mnt/wslg/.X11-unix/X0").exists():
        display_name = ":0"
    if not display_name:
        parser.error("DISPLAY is unset and WSLg is unavailable")

    x11 = ctypes.CDLL(ctypes.util.find_library("X11") or "libX11.so.6")
    display_pointer = ctypes.c_void_p
    window_type = ctypes.c_ulong
    window_pointer = ctypes.POINTER(window_type)
    integer_pointer = ctypes.POINTER(ctypes.c_int)
    count_pointer = ctypes.POINTER(ctypes.c_ulong)
    byte_pointer = ctypes.POINTER(ctypes.c_ubyte)
    signatures = {
        "XOpenDisplay": ([ctypes.c_char_p], display_pointer),
        "XDefaultRootWindow": ([display_pointer], window_type),
        "XInternAtom": ([display_pointer, ctypes.c_char_p, ctypes.c_int], window_type),
        "XQueryTree": ([display_pointer, window_type, window_pointer, window_pointer, ctypes.POINTER(window_pointer), ctypes.POINTER(ctypes.c_uint)], ctypes.c_int),
        "XGetWindowProperty": ([display_pointer, window_type, window_type, ctypes.c_long, ctypes.c_long, ctypes.c_int, window_type, window_pointer, integer_pointer, count_pointer, count_pointer, ctypes.POINTER(byte_pointer)], ctypes.c_int),
        "XResizeWindow": ([display_pointer, window_type, ctypes.c_uint, ctypes.c_uint], ctypes.c_int),
        "XRaiseWindow": ([display_pointer, window_type], ctypes.c_int),
        "XSetInputFocus": ([display_pointer, window_type, ctypes.c_int, ctypes.c_ulong], ctypes.c_int),
        "XTranslateCoordinates": ([display_pointer, window_type, window_type, ctypes.c_int, ctypes.c_int, integer_pointer, integer_pointer, window_pointer], ctypes.c_int),
        "XSync": ([display_pointer, ctypes.c_int], ctypes.c_int),
        "XFree": ([ctypes.c_void_p], ctypes.c_int),
        "XCloseDisplay": ([display_pointer], ctypes.c_int),
    }
    for name, (arguments, result) in signatures.items():
        function = getattr(x11, name)
        function.argtypes = arguments
        function.restype = result

    display = x11.XOpenDisplay(display_name.encode())
    if not display:
        parser.error(f"cannot connect to X11 display {display_name}")
    try:
        pid_atom = x11.XInternAtom(display, b"_NET_WM_PID", False)
        pending = [x11.XDefaultRootWindow(display)]
        window = None
        while pending:
            candidate = pending.pop()
            actual_type, actual_format = window_type(), ctypes.c_int()
            count, remaining, value = ctypes.c_ulong(), ctypes.c_ulong(), byte_pointer()
            x11.XGetWindowProperty(display, candidate, pid_atom, 0, 1, False, 0, ctypes.byref(actual_type), ctypes.byref(actual_format), ctypes.byref(count), ctypes.byref(remaining), ctypes.byref(value))
            pid = ctypes.cast(value, window_pointer)[0] if value and count.value and actual_format.value == 32 else None
            if value:
                x11.XFree(value)
            if pid == args.pid:
                window = candidate
                break
            root, parent, children, child_count = window_type(), window_type(), window_pointer(), ctypes.c_uint()
            if x11.XQueryTree(display, candidate, ctypes.byref(root), ctypes.byref(parent), ctypes.byref(children), ctypes.byref(child_count)):
                pending.extend(children[index] for index in range(child_count.value))
                if children:
                    x11.XFree(children)
        if window is None:
            parser.error(f"no X11 window found for process {args.pid}; wait for startup and force X11 with WAYLAND_DISPLAY= DISPLAY={display_name}")
        if args.size:
            if min(args.size) <= 0:
                parser.error("window dimensions must be positive")
            x11.XResizeWindow(display, window, *args.size)
            x11.XSync(display, False)
        time.sleep(args.delay)
        if args.click or args.scroll:
            xtest = ctypes.CDLL(ctypes.util.find_library("Xtst") or "libXtst.so.6")
            xtest.XTestFakeMotionEvent.argtypes = [display_pointer, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_ulong]
            xtest.XTestFakeButtonEvent.argtypes = [display_pointer, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
            x11.XRaiseWindow(display, window)
            x11.XSetInputFocus(display, window, 2, 0)
            coordinates = args.click or args.scroll[:2]
            root_x, root_y, child = ctypes.c_int(), ctypes.c_int(), window_type()
            x11.XTranslateCoordinates(display, window, x11.XDefaultRootWindow(display), *coordinates, ctypes.byref(root_x), ctypes.byref(root_y), ctypes.byref(child))
            xtest.XTestFakeMotionEvent(display, -1, root_x.value, root_y.value, 0)
            x11.XSync(display, False)
            time.sleep(0.2)

            def press(button):
                xtest.XTestFakeButtonEvent(display, button, True, 0)
                xtest.XTestFakeButtonEvent(display, button, False, 0)
                x11.XSync(display, False)
                time.sleep(0.08)

            if args.click:
                press(1)
            if args.scroll:
                for amount, negative_button, positive_button in [(args.scroll[2], 6, 7), (args.scroll[3], 4, 5)]:
                    for _ in range(abs(amount)):
                        press(positive_button if amount > 0 else negative_button)
            time.sleep(0.5)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run([
            "ffmpeg", "-hide_banner", "-loglevel", "error", "-y",
            "-f", "x11grab", "-draw_mouse", "0", "-window_id", str(window),
            "-i", display_name, "-frames:v", "1", "-update", "1", str(args.output),
        ], check=True, timeout=20)
        print(f"Captured window {window} to {args.output.resolve()}")
    finally:
        x11.XCloseDisplay(display)


if __name__ == "__main__":
    main()
