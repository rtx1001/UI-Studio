# UI Studio

<p align="center">
  <img src="logo_Ui.png" width="96" alt="UI Studio app icon">
</p>

A portable Windows tool for batch-reskinning UI image assets with a consistent visual style.

It provides folder-based input and output browsing, style and palette controls, batch processing, and preservation of source filenames, folder structure, image dimensions, and transparency.

## Screenshots

<p align="center">
  <img src="screenshot/scr_1.jpg" alt="UI Studio workspace" width="100%">
</p>

<table>
  <tr>
    <td width="50%"><img src="screenshot/scr_2.jpg" alt="Processed output browser"></td>
    <td width="50%"><img src="screenshot/scr_3.jpg" alt="Before and after comparison"></td>
  </tr>
  <tr>
    <td colspan="2"><img src="screenshot/scr_4.jpg" alt="PC compatibility check"></td>
  </tr>
</table>

## Run from source

```powershell
npm install
npm run tauri dev
```

## Portable build

```powershell
npm run tauri -- build --no-bundle
./tools/build_portable.ps1
```

The portable package starts with blank settings and downloads required processing resources through the app after checking system compatibility.
