---
title: FilmCraft does not open on Windows
description: Fix FilmCraft flashing in Task Manager or quitting right after launch on Windows 11, caused by a graphics driver crash, by choosing another graphics backend.
order: 10
verified: 0.6.0
---

Update your graphics driver. If FilmCraft still quits, start it from PowerShell with another graphics backend, as shown below.

## Symptoms

- You double-click FilmCraft and nothing appears, or it shows up in Task Manager for a moment and quits.
- Windows Event Viewer (Windows Logs ▸ Application) shows an access violation (`0xc0000005`) in a graphics driver file. On Intel graphics this was `igvk64.dll`, in the Vulkan driver. On some AMD systems it was `atio6axx.dll`.

## Fix

1. Update your graphics driver from your GPU maker. This may not be enough: the Intel laptop in issue 1037 crashed the same way on the latest Intel driver.
2. If it still does not open, set `WGPU_BACKEND` to pick the graphics backend. Open PowerShell, go to the folder that holds `filmcraft.exe`, and run:

        $env:WGPU_BACKEND = "dx12"
        & .\filmcraft.exe

   With `dx12`, FilmCraft may start but run some effects on the CPU instead of the GPU.

3. If `dx12` does not start it either: on AMD or NVIDIA graphics, try `vulkan` in the same two lines. On Intel graphics skip `vulkan`, which is the driver that crashes there, and try `gl`.
4. The setting applies to this PowerShell window only. To undo it, run `Remove-Item Env:WGPU_BACKEND`.

Automatic switching to a safer graphics mode after a failed start was added after 0.6.0 and is not in a release yet ([issue 820](https://github.com/storytold/filmcraft/issues/820)).

On one Intel UHD Graphics 620 laptop with Windows 11, Vulkan crashed at startup, `dx12` started but fell back to the CPU compositor, and `gl` started cleanly with GPU compositing. The workaround that user reported was `setx WGPU_BACKEND gl`, which keeps the setting for later launches. Try `gl` only after `dx12` and `vulkan` fail: on some AMD drivers an OpenGL start can bring the crash back.

## Why this happens

By default FilmCraft starts only Vulkan and DirectX 12 on Windows, not OpenGL. Some graphics drivers crash inside the driver while the window is being created, before FilmCraft can handle an error. Intel's Vulkan driver and AMD's OpenGL driver have both done this.

## Still stuck?

[Report a problem](send-a-bug-report.md) and include your `filmcraft.log`. Or ask in [Discord](https://discord.gg/artcraft). Related reports are [issue 1037](https://github.com/storytold/filmcraft/issues/1037), a driver crash on Intel graphics, and [issue 1134](https://github.com/storytold/filmcraft/issues/1134), FilmCraft not opening on Windows 11.
