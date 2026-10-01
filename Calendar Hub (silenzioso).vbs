' Avvia Calendar Hub senza finestra del prompt (uso quotidiano).
Set sh = CreateObject("WScript.Shell")
appDir = Left(WScript.ScriptFullName, InStrRev(WScript.ScriptFullName, "\"))
Dim fso : Set fso = CreateObject("Scripting.FileSystemObject")

' Preferisce l'ambiente virtuale dell'app: e' li' che sta pywin32.
py = appDir & ".venv\Scripts\pythonw.exe"
If Not fso.FileExists(py) Then py = sh.ExpandEnvironmentStrings("%LOCALAPPDATA%") & "\Programs\Python\Python312\pythonw.exe"
If Not fso.FileExists(py) Then py = "pythonw"

sh.CurrentDirectory = appDir
sh.Run """" & py & """ """ & appDir & "avvia.pyw""", 0, False

