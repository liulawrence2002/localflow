Set shell = CreateObject("WScript.Shell")
Set filesystem = CreateObject("Scripting.FileSystemObject")

scriptDirectory = filesystem.GetParentFolderName(WScript.ScriptFullName)
powershellScript = filesystem.BuildPath(scriptDirectory, "Start-LocalFlow.ps1")
command = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File """ & powershellScript & """"

shell.Run command, 0, False
