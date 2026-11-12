// Native, owned-fixture reparse mutation used to test the publication race boundary.
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;

public static class ShortcutReparseFixture {
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern SafeFileHandle CreateFileW(string path, uint access, uint share,
        IntPtr security, uint disposition, uint flags, IntPtr template);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool DeviceIoControl(SafeFileHandle handle, uint code, byte[] input,
        uint length, IntPtr output, uint capacity, out uint returned, IntPtr overlapped);
    public static void SetJunction(string path, string target, bool remove) {
        using (SafeFileHandle handle = CreateFileW(path, 0x40000000, 3,
            IntPtr.Zero, 3, 0x02200000, IntPtr.Zero)) {
            if (handle.IsInvalid) throw new Win32Exception(Marshal.GetLastWin32Error());
            byte[] data;
            if (remove) data = new byte[8];
            else {
                byte[] substitute = Encoding.Unicode.GetBytes("\\??\\" + target);
                byte[] print = Encoding.Unicode.GetBytes(target);
                data = new byte[16 + substitute.Length + print.Length + 4];
                BitConverter.GetBytes((ushort)(data.Length - 8)).CopyTo(data, 4);
                BitConverter.GetBytes((ushort)substitute.Length).CopyTo(data, 10);
                BitConverter.GetBytes((ushort)(substitute.Length + 2)).CopyTo(data, 12);
                BitConverter.GetBytes((ushort)print.Length).CopyTo(data, 14);
                substitute.CopyTo(data, 16);
                print.CopyTo(data, 18 + substitute.Length);
            }
            BitConverter.GetBytes(0xa0000003U).CopyTo(data, 0);
            uint returned;
            if (!DeviceIoControl(handle, remove ? 0x900acU : 0x900a4U, data,
                (uint)data.Length, IntPtr.Zero, 0, out returned, IntPtr.Zero))
                throw new Win32Exception(Marshal.GetLastWin32Error());
        }
    }
}
