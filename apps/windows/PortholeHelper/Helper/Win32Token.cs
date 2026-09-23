namespace Porthole.WindowsHelper;

// Values from the Windows SDK's winnt.h and winerror.h.
internal static class Win32Token
{
    internal const uint Query = 0x0008; // TOKEN_QUERY
    internal const int Elevation = 20; // TokenElevation
    internal const int LogonSid = 28; // TokenLogonSid
    internal const uint LogonIdGroup = 0xC0000000; // SE_GROUP_LOGON_ID
    internal const int InsufficientBuffer = 122; // ERROR_INSUFFICIENT_BUFFER
}
