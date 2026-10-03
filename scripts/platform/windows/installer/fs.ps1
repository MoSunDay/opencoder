# Windows creation-time ACLs and durable journal replacement.
if (-not ('OpenCoderInstallFiles' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using System.Security.Principal;
using Microsoft.Win32.SafeHandles;
public static class OpenCoderInstallFiles {
    [StructLayout(LayoutKind.Sequential)] struct SecurityAttributes {
        public int Length; public IntPtr Descriptor; public int Inherit;
    }
    [DllImport("advapi32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern bool ConvertStringSecurityDescriptorToSecurityDescriptor(string s, uint revision, out IntPtr p, out uint length);
    [DllImport("kernel32.dll")] static extern IntPtr LocalFree(IntPtr p);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern bool CreateDirectory(string path, ref SecurityAttributes attributes);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern SafeFileHandle CreateFile(string path, uint access, uint share, ref SecurityAttributes attributes, uint creation, uint flags, IntPtr template);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern bool MoveFileEx(string source, string destination, uint flags);
    static string NativePath(string path) {
        string full=Path.GetFullPath(path);
        if(full.StartsWith(@"\\?\")) return full;
        return full.StartsWith(@"\\") ? @"\\?\UNC\"+full.Substring(2) : @"\\?\"+full;
    }
    static SecurityAttributes Attributes() {
        string sid=WindowsIdentity.GetCurrent().User.Value;
        IntPtr descriptor; uint length;
        string s="O:"+sid+"D:P(A;OICI;FA;;;"+sid+")(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)";
        if(!ConvertStringSecurityDescriptorToSecurityDescriptor(s,1,out descriptor,out length)) throw new Win32Exception();
        return new SecurityAttributes { Length=Marshal.SizeOf<SecurityAttributes>(), Descriptor=descriptor };
    }
    public static void Directory(string path) {
        var a=Attributes();
        try { if(!CreateDirectory(NativePath(path),ref a) && Marshal.GetLastWin32Error()!=183) throw new Win32Exception(); }
        finally { LocalFree(a.Descriptor); }
    }
    static FileStream Open(string path, uint creation) {
        var a=Attributes(); SafeFileHandle handle;
        try { handle=CreateFile(NativePath(path),0xC0000000,0,ref a,creation,0x80200000,IntPtr.Zero); }
        finally { LocalFree(a.Descriptor); }
        if(handle.IsInvalid) { int error=Marshal.GetLastWin32Error(); handle.Dispose(); throw new Win32Exception(error); }
        return new FileStream(handle,FileAccess.ReadWrite,4096,false);
    }
    public static FileStream Lock(string path) { return Open(path,4); }
    public static void Write(string path, byte[] bytes) {
        using(var file=Open(path,1)) { file.Write(bytes,0,bytes.Length); file.Flush(true); }
    }
    public static void Replace(string source,string destination) {
        if(!MoveFileEx(NativePath(source),NativePath(destination),9)) throw new Win32Exception();
    }
    public static void MoveDirectory(string source,string destination) {
        if(!MoveFileEx(NativePath(source),NativePath(destination),8)) throw new Win32Exception();
    }
    public static void Copy(string source,string destination) {
        using(var input=new FileStream(source,FileMode.Open,FileAccess.Read,FileShare.Read))
        using(var output=Open(destination,1)) { input.CopyTo(output); output.Flush(true); }
    }
}
'@
}
function Assert-InstallPath([string]$Path) {
    $current = [IO.Path]::GetFullPath($Path)
    while ($current) {
        if (Test-Path -LiteralPath $current) {
            if ((Get-Item -LiteralPath $current -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) {
                throw "Installation paths must not contain links: $current"
            }
        }
        $parent = Split-Path $current -Parent
        if ($parent -eq $current) { break }
        $current = $parent
    }
}
function Assert-InstallPrivate([string]$Path) {
    Assert-InstallPath $Path
    $acl = Get-Acl -LiteralPath $Path
    $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    if (-not $acl.AreAccessRulesProtected) { throw 'Installation journal must have a protected ACL.' }
    if ($acl.GetOwner([Security.Principal.SecurityIdentifier]).Value -notin @($sid, 'S-1-5-18', 'S-1-5-32-544')) { throw 'Unexpected installation journal owner.' }
    foreach ($rule in $acl.Access) {
        $identity = $rule.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value
        if ($rule.AccessControlType -eq 'Allow' -and $identity -notin @($sid, 'S-1-5-18', 'S-1-5-32-544')) {
            throw 'Installation journal grants access to another identity.'
        }
    }
}
function New-InstallPrivateDirectory([string]$Path) {
    Assert-InstallPath $Path
    [OpenCoderInstallFiles]::Directory($Path)
    if (-not (Test-Path -LiteralPath $Path -PathType Container)) { throw 'Expected an installation directory.' }
    Assert-InstallPrivate $Path
}
function Write-InstallJson([string]$Path, $Value, [switch]$CreateOnly) {
    if ($CreateOnly -and (Test-Path -LiteralPath $Path)) { throw 'The installation before-image already exists.' }
    if (Test-Path -LiteralPath $Path) { Assert-InstallPrivate $Path }
    $temporary = "$Path.new-$([guid]::NewGuid())"
    try {
        $bytes = [Text.UTF8Encoding]::new($false).GetBytes(($Value | ConvertTo-Json -Depth 20 -Compress))
        if ($CreateOnly) { [OpenCoderInstallFiles]::Write($Path, $bytes); return }
        [OpenCoderInstallFiles]::Write($temporary, $bytes)
        [OpenCoderInstallFiles]::Replace($temporary, $Path)
    } finally {
        if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force }
    }
}
function Move-InstallDirectory([string]$Source, [string]$Destination) {
    Assert-InstallPath $Source
    Assert-InstallPath $Destination
    if (Test-Path -LiteralPath $Destination) { throw 'The installation rename target already exists.' }
    [OpenCoderInstallFiles]::MoveDirectory($Source, $Destination)
}
function Read-InstallJson([string]$Path) {
    Assert-InstallPrivate $Path
    Get-Content -Raw -LiteralPath $Path | ConvertFrom-Json -AsHashtable
}
