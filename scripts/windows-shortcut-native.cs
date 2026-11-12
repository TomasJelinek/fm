// Windows-only namespace pinning and same-volume shortcut publication.
// P/Invokes use Unicode and SafeFileHandle ownership; no Shell code runs here.
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;
using Microsoft.Win32.SafeHandles;

namespace MidnightShortcut {
    [ComImport, Guid("00021401-0000-0000-C000-000000000046")]
    class ShellLinkObject { }
    [ComImport, Guid("000214F9-0000-0000-C000-000000000046"),
        InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface ShellLinkW {
        void GetPath([Out, MarshalAs(UnmanagedType.LPWStr)] StringBuilder path, int capacity,
            IntPtr findData, uint flags);
        void GetIDList(out IntPtr list);
        void SetIDList(IntPtr list);
        void GetDescription([Out, MarshalAs(UnmanagedType.LPWStr)] StringBuilder text, int capacity);
        void SetDescription([MarshalAs(UnmanagedType.LPWStr)] string text);
        void GetWorkingDirectory([Out, MarshalAs(UnmanagedType.LPWStr)] StringBuilder path, int capacity);
        void SetWorkingDirectory([MarshalAs(UnmanagedType.LPWStr)] string path);
        void GetArguments([Out, MarshalAs(UnmanagedType.LPWStr)] StringBuilder text, int capacity);
        void SetArguments([MarshalAs(UnmanagedType.LPWStr)] string text);
        void GetHotkey(out short key);
        void SetHotkey(short key);
        void GetShowCmd(out int command);
        void SetShowCmd(int command);
        void GetIconLocation([Out, MarshalAs(UnmanagedType.LPWStr)] StringBuilder path, int capacity, out int index);
        void SetIconLocation([MarshalAs(UnmanagedType.LPWStr)] string path, int index);
        void SetRelativePath([MarshalAs(UnmanagedType.LPWStr)] string path, uint reserved);
        void Resolve(IntPtr window, uint flags);
        void SetPath([MarshalAs(UnmanagedType.LPWStr)] string path);
    }
    public sealed class DirectoryPins : IDisposable {
        internal readonly List<SafeFileHandle> Handles = new List<SafeFileHandle>();
        public readonly List<string> Paths = new List<string>();
        public uint Tag { get; internal set; }
        public uint Volume { get; internal set; }
        internal SafeFileHandle Leaf { get { return Handles[Handles.Count - 1]; } }
        public void Dispose() {
            for (int index = Handles.Count - 1; index >= 0; index--) Handles[index].Dispose();
            Handles.Clear();
        }
    }

    public static class Native {
        public static void CreateShortcut(string temporary, string executable, string workingDirectory) {
            object instance = new ShellLinkObject();
            try {
                ShellLinkW link = (ShellLinkW)instance;
                link.SetPath(executable);
                link.SetWorkingDirectory(workingDirectory);
                link.SetArguments("");
                link.SetIconLocation(executable, 0);
                link.SetDescription("Midnight Explorer - private verified installation");
                ((System.Runtime.InteropServices.ComTypes.IPersistFile)instance).Save(temporary, true);
            } finally { Marshal.FinalReleaseComObject(instance); }
        }
        public static string[] ReadShortcut(string path) {
            object instance = new ShellLinkObject();
            try {
                ((System.Runtime.InteropServices.ComTypes.IPersistFile)instance).Load(path, 0);
                ShellLinkW link = (ShellLinkW)instance;
                // Shell link APIs have native fixed output buffers; no filesystem path is
                // reconstructed from this verification-only display readback.
                StringBuilder target = new StringBuilder(32768);
                StringBuilder icon = new StringBuilder(32768);
                StringBuilder cwd = new StringBuilder(32768);
                StringBuilder arguments = new StringBuilder(32768);
                int iconIndex;
                link.GetPath(target, target.Capacity, IntPtr.Zero, 4);
                link.GetIconLocation(icon, icon.Capacity, out iconIndex);
                link.GetWorkingDirectory(cwd, cwd.Capacity);
                link.GetArguments(arguments, arguments.Capacity);
                return new string[] { target.ToString(), icon.ToString() + "," + iconIndex,
                    cwd.ToString(), arguments.ToString() };
            } finally { Marshal.FinalReleaseComObject(instance); }
        }
        [StructLayout(LayoutKind.Sequential)]
        struct AttributeTag { public uint Attributes; public uint Tag; }
        [StructLayout(LayoutKind.Sequential)]
        struct Information {
            public uint Attributes;
            public System.Runtime.InteropServices.ComTypes.FILETIME Creation, Access, Write;
            public uint Volume, SizeHigh, SizeLow, Links, IndexHigh, IndexLow;
        }
        [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
        struct RenameInfo {
            [MarshalAs(UnmanagedType.U1)] public bool Replace;
            public IntPtr Root;
            public uint Length;
            public char Name;
        }
        [StructLayout(LayoutKind.Sequential)]
        struct IoStatus { public IntPtr Status; public UIntPtr Information; }
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        static extern SafeFileHandle CreateFileW(string path, uint access, uint share,
            IntPtr security, uint disposition, uint flags, IntPtr template);
        [DllImport("kernel32.dll", SetLastError = true)]
        static extern bool GetFileInformationByHandleEx(SafeFileHandle handle, int kind,
            out AttributeTag information, uint size);
        [DllImport("kernel32.dll", SetLastError = true)]
        static extern bool GetFileInformationByHandle(SafeFileHandle handle, out Information information);
        [DllImport("ntdll.dll")]
        static extern int NtSetInformationFile(SafeFileHandle handle, out IoStatus status,
            IntPtr information, uint size, int kind);
        [DllImport("ntdll.dll")]
        static extern uint RtlNtStatusToDosError(int status);

        public static bool IsCloudTag(uint tag) {
            // Only documented CLOUD/CLOUD_1..F. In particular, no name surrogate.
            return (tag & 0xffff0fffU) == 0x9000001aU;
        }
        public static string LocalPath(string path) {
            if (String.IsNullOrEmpty(path) || path.Length < 3 ||
                !Char.IsLetter(path[0]) || path[1] != ':' || (path[2] != '\\' && path[2] != '/'))
                throw new IOException("Shortcut paths must be absolute local drive paths.");
            path = path.Replace('/', '\\');
            foreach (string part in path.Substring(3).Split('\\')) {
                if (part == "." || part == ".." || part.IndexOf(':') >= 0 ||
                    part.EndsWith(".", StringComparison.Ordinal) || part.EndsWith(" ", StringComparison.Ordinal))
                    throw new IOException("Ambiguous shortcut path component.");
            }
            string full = Path.GetFullPath(path);
            return full.Length == 3 ? full : full.TrimEnd('\\');
        }
        static SafeFileHandle Open(string path, uint access, uint share) {
            // OPEN_REPARSE_POINT + BACKUP_SEMANTICS: inspect the entry itself.
            SafeFileHandle handle = CreateFileW("\\\\?\\" + path, access, share,
                IntPtr.Zero, 3, 0x02200000, IntPtr.Zero);
            if (handle.IsInvalid) {
                int error = Marshal.GetLastWin32Error();
                handle.Dispose();
                throw new Win32Exception(error, "Cannot pin shortcut path: " + path);
            }
            return handle;
        }
        static AttributeTag Inspect(SafeFileHandle handle) {
            AttributeTag info;
            if (!GetFileInformationByHandleEx(handle, 9, out info, 8))
                throw new Win32Exception(Marshal.GetLastWin32Error());
            return info;
        }
        static Information Identity(SafeFileHandle handle) {
            Information info;
            if (!GetFileInformationByHandle(handle, out info))
                throw new Win32Exception(Marshal.GetLastWin32Error());
            return info;
        }
        public static DirectoryPins PinDirectory(string path, bool allowCloud) {
            path = LocalPath(path);
            DirectoryPins pins = new DirectoryPins();
            try {
                string current = path.Substring(0, 3);
                List<string> components = new List<string>();
                components.Add(current);
                foreach (string part in path.Substring(3).Split('\\')) {
                    if (part.Length == 0) continue;
                    current = Path.Combine(current, part);
                    components.Add(current);
                }
                foreach (string component in components) {
                    // Retain every parent before opening its child; deny write/delete sharing.
                    // The target leaf needs write sharing for the kernel's rename-directory open.
                    // Publication addresses that retained handle, so a later leaf reparse cannot
                    // redirect the rename. Ancestors still deny write sharing; all deny delete.
                    uint share = allowCloud && component == path ? 3U : 1U;
                    SafeFileHandle handle = Open(component, 0xa0, share);
                    pins.Handles.Add(handle);
                    AttributeTag info = Inspect(handle);
                    if ((info.Attributes & 0x10) == 0 || ((info.Attributes & 0x400) != 0 &&
                        (!allowCloud || !IsCloudTag(info.Tag))))
                        throw new IOException("Unsupported shortcut directory/reparse tag: " + component);
                    pins.Paths.Add(component);
                    pins.Tag = info.Tag;
                    pins.Volume = Identity(handle).Volume;
                }
                return pins;
            } catch { pins.Dispose(); throw; }
        }
        public static FileStream OpenExecutable(string path) {
            SafeFileHandle handle = Open(LocalPath(path), 0x80000080, 1);
            try {
                AttributeTag info = Inspect(handle);
                if ((info.Attributes & (0x10 | 0x400)) != 0)
                    throw new IOException("Runtime executable must be a plain file.");
                return new FileStream(handle, FileAccess.Read);
            } catch { handle.Dispose(); throw; }
        }
        public static void ValidateDestination(string path) {
            SafeFileHandle handle = CreateFileW("\\\\?\\" + LocalPath(path), 0x80, 7,
                IntPtr.Zero, 3, 0x02200000, IntPtr.Zero);
            if (handle.IsInvalid) {
                int error = Marshal.GetLastWin32Error();
                handle.Dispose();
                if (error == 2) return;
                throw new Win32Exception(error, "Cannot inspect existing shortcut.");
            }
            using (handle) {
                AttributeTag info = Inspect(handle);
                if ((info.Attributes & 0x10) != 0 || ((info.Attributes & 0x400) != 0 && !IsCloudTag(info.Tag)))
                    throw new IOException("Existing shortcut is a directory, link or unknown reparse entry.");
            }
        }
        public static void Publish(string source, DirectoryPins destination, string name, bool replace) {
            if (String.IsNullOrEmpty(name) || name.IndexOfAny(new char[] {'\\', '/', ':'}) >= 0 ||
                name == "." || name == "..") throw new IOException("Expected one shortcut filename.");
            using (SafeFileHandle handle = Open(LocalPath(source), 0x80010080, 1)) {
                AttributeTag info = Inspect(handle);
                if ((info.Attributes & (0x10 | 0x400)) != 0)
                    throw new IOException("Staged shortcut must be a plain file.");
                if (Identity(handle).Volume != destination.Volume)
                    throw new IOException("Shortcut publication requires the same volume; no copy fallback.");
                byte[] bytes = Encoding.Unicode.GetBytes(name);
                int nameOffset = (int)Marshal.OffsetOf(typeof(RenameInfo), "Name");
                int size = checked(24 + bytes.Length);
                IntPtr buffer = Marshal.AllocHGlobal(size);
                try {
                    for (int index = 0; index < size; index++) Marshal.WriteByte(buffer, index, 0);
                    Marshal.WriteByte(buffer, 0, replace ? (byte)1 : (byte)0);
                    Marshal.WriteIntPtr(buffer, (int)Marshal.OffsetOf(typeof(RenameInfo), "Root"),
                        destination.Leaf.DangerousGetHandle());
                    Marshal.WriteInt32(buffer, (int)Marshal.OffsetOf(typeof(RenameInfo), "Length"), bytes.Length);
                    Marshal.Copy(bytes, 0, IntPtr.Add(buffer, nameOffset), bytes.Length);
                    // FileRenameInformation changes the directory entry, never writes through it.
                    // Source and destination handles remain live throughout the synchronous native call.
                    IoStatus completion;
                    int status = NtSetInformationFile(handle, out completion, buffer, (uint)size, 10);
                    if (status < 0) {
                        int error = unchecked((int)RtlNtStatusToDosError(status));
                        throw new Win32Exception(error, "Shortcut rename failed: " + error);
                    }
                } finally { Marshal.FreeHGlobal(buffer); }
            }
        }
    }
}
