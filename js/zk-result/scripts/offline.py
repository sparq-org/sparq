"""Run a development check with outbound network syscalls denied on Linux."""
import ctypes
import errno
import os
import socket
import sys

lib = ctypes.CDLL("libseccomp.so.2", use_errno=True)
lib.seccomp_init.restype = ctypes.c_void_p
lib.seccomp_rule_add.argtypes = [ctypes.c_void_p, ctypes.c_uint32, ctypes.c_int, ctypes.c_uint]
lib.seccomp_load.argtypes = [ctypes.c_void_p]
lib.seccomp_syscall_resolve_name.argtypes = [ctypes.c_char_p]
context = lib.seccomp_init(0x7FFF0000)
if not context:
    raise RuntimeError("cannot create network isolation filter")
for name in (b"connect", b"sendto", b"sendmsg", b"sendmmsg"):
    number = lib.seccomp_syscall_resolve_name(name)
    if number < 0 or lib.seccomp_rule_add(context, 0x00050000 | errno.EPERM, number, 0) != 0:
        raise RuntimeError("cannot deny network syscall: " + name.decode())
if lib.seccomp_load(context) != 0:
    raise RuntimeError("cannot enforce network isolation filter")
with socket.socket() as probe:
    try:
        probe.connect(("127.0.0.1", 9))
    except OSError as error:
        if error.errno != errno.EPERM:
            raise RuntimeError("network isolation probe did not receive EPERM") from error
    else:
        raise RuntimeError("network isolation probe unexpectedly connected")
os.execvp(sys.argv[1], sys.argv[1:])
