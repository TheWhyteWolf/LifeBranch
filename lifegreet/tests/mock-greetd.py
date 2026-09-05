#!/usr/bin/env python3
"""Dev-only mock of the greetd IPC socket (never installed).

Speaks the real wire protocol: native-endian u32 length + JSON payload.
Password "ok" authenticates; anything else fails after a PAM-ish delay.

Rehearse the full greeter loop inside niri:
    python tests/mock-greetd.py /tmp/mock-greetd.sock &
    GREETD_SOCK=/tmp/mock-greetd.sock cargo run
or nested under cage (closest to the real thing):
    GREETD_SOCK=/tmp/mock-greetd.sock cage -s -d -- ./target/debug/lifegreet

Flags:
    --info      send an Info message ("Last login...") before Success
    --two-step  demand a second Secret prompt after the password (2FA shape;
                the greeter should fail gracefully and cancel)
"""
import json
import os
import struct
import socket
import sys
import time


def send(conn, obj):
    data = json.dumps(obj).encode()
    conn.sendall(struct.pack("=I", len(data)) + data)
    print(f"mock-greetd <- {obj}", flush=True)


def recv(conn):
    hdr = conn.recv(4, socket.MSG_WAITALL)
    if len(hdr) < 4:
        return None
    (n,) = struct.unpack("=I", hdr)
    obj = json.loads(conn.recv(n, socket.MSG_WAITALL).decode())
    print(f"mock-greetd -> {obj}", flush=True)
    return obj


def serve(conn, info, two_step):
    session = None  # username of the created session
    asked = 0       # secret prompts answered this session
    while (req := recv(conn)) is not None:
        t = req["type"]
        if t == "create_session":
            session, asked = req["username"], 0
            send(conn, {"type": "auth_message",
                        "auth_message_type": "secret",
                        "auth_message": "Password: "})
        elif t == "post_auth_message_response":
            time.sleep(0.8)  # pam_unix-ish delay
            if session is None:
                send(conn, {"type": "error", "error_type": "error",
                            "description": "no session"})
                continue
            asked += 1
            if req.get("response") != "ok":
                session = None
                send(conn, {"type": "error", "error_type": "auth_error",
                            "description": "authentication failed"})
            elif two_step and asked == 1:
                send(conn, {"type": "auth_message",
                            "auth_message_type": "secret",
                            "auth_message": "Second factor: "})
            elif info and asked == 1:
                # Display-only message; expects an empty ack, then succeed.
                send(conn, {"type": "auth_message",
                            "auth_message_type": "info",
                            "auth_message": "Last login: yesterday"})
                ack = recv(conn)
                assert ack and ack.get("response") is None
                send(conn, {"type": "success"})
            else:
                send(conn, {"type": "success"})
        elif t == "start_session":
            print(f"mock-greetd: WOULD START {req['cmd']} for {session!r}", flush=True)
            send(conn, {"type": "success"})
        elif t == "cancel_session":
            session = None
            send(conn, {"type": "success"})
        else:
            send(conn, {"type": "error", "error_type": "error",
                        "description": f"unknown request {t}"})


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    path = args[0] if args else "/tmp/mock-greetd.sock"
    info = "--info" in sys.argv
    two_step = "--two-step" in sys.argv
    try:
        os.unlink(path)
    except FileNotFoundError:
        pass
    srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    srv.bind(path)
    srv.listen(1)
    print(f"mock-greetd: listening on {path}", flush=True)
    while True:
        conn, _ = srv.accept()
        print("mock-greetd: client connected", flush=True)
        try:
            serve(conn, info, two_step)
        except (ConnectionError, AssertionError) as e:
            print(f"mock-greetd: {e}", flush=True)
        finally:
            conn.close()
            print("mock-greetd: client gone", flush=True)


if __name__ == "__main__":
    main()
