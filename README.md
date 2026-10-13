# ScreenShotCM

Capturas de pantalla y anotación para Windows 10/11. Un agente residente en el área de notificación: arranca con Windows, vive en
la bandeja y responde a atajos globales.

## Instalar

Descarga el instalador y ábrelo:

- Siempre la última versión:
  [ScreenShotCM.msi](https://github.com/cmonroyp/screenshotcm-web/releases/latest/download/ScreenShotCM.msi),
  desde el sitio público <https://cmonroyp.github.io/screenshotcm-web/>.
- Una versión concreta: en [Releases](https://github.com/cmonroyp/screenshotcm-web/releases)
  del repositorio del sitio, cada etiqueta (`v0.3.0`, …) lleva su
  `ScreenShotCM-<versión>.msi` y su SHA-256.

Este repositorio es privado; el instalador se publica en el del sitio, que es
público. Se
instala para tu usuario, sin pedir administrador, en
`%LOCALAPPDATA%\Programs\ScreenShotCM`; deja el agente en la bandeja y
arrancando con Windows. Una versión nueva se instala encima de la anterior:
el instalador cierra el agente, reemplaza el programa y lo vuelve a abrir.
Se desinstala desde *Configuración → Aplicaciones*; tus capturas y ajustes se
conservan.

## Qué hace

**Capturar** (atajos por defecto, cambiables en *Ajustes*):

| | |
|---|---|
| Área | Impr Pant |
| Ventana | Shift+Impr Pant (o Espacio durante la selección) |
| Pantalla completa | Ctrl+Impr Pant |
| Área anterior | Alt+Shift+Impr Pant |
| Temporizador | menú de la bandeja |

La pantalla se congela y se muestra con una cruz, las coordenadas y una lupa.
Al arrastrar se ve el tamaño; Shift hace la selección cuadrada, las flechas
mueven el puntero de píxel en píxel, Esc o clic derecho cancelan.

**Después de capturar** la imagen va al portapapeles y al historial, y aparece
la miniatura de acceso rápido abajo a la izquierda: al pasar el ratón ofrece
Copiar y Guardar, y en las esquinas cerrar, fijar, anotar y eliminar.
Arrastrarla suelta la imagen en otro programa; doble clic abre el editor.

**El editor**: recorte, fondo (degradados o colores, margen, redondeo y
sombra), selección, rectángulo, rectángulo relleno, elipse, línea, flecha,
texto (normal o etiqueta), pixelado o desenfoque, foco, contador, lápiz y
resaltador, con color y tamaño. Deshacer y rehacer, zoom, *Guardar como…*,
*Listo* (guarda las anotaciones en la captura), y abajo *Arrastrar*, Fijar,
Copiar y Guardar. Atajos: V R F O L A T B S N P H C para las herramientas,
Ctrl+Z / Ctrl+Y, Ctrl+C, Ctrl+S, Supr, Intro, Ctrl+rueda para el zoom.

**Capturas fijadas**: flotan sobre todo; se arrastran, la rueda cambia el
tamaño, Ctrl+rueda la opacidad, las flechas las mueven; el menú del clic
derecho copia, guarda, anota y las bloquea (los clics las atraviesan).

**Historial**: las capturas del último mes, con copiar, anotar, fijar y
eliminar. **Abrir del portapapeles** y **Anotar una imagen…** abren
cualquier imagen en el editor.

## Medidas

| | ScreenShotCM 0.3.0 |
|---|---|
| Instalador | 432 KB |
| Programa | un solo `.exe`, sin DLL propias |
| Memoria privada en reposo | 1,8 MB |
| Hilos en reposo | 2 |

Medido con el agente instalado y en reposo en la bandeja (`Get-Process`,
2026-10-13). Las ventanas dibujan con Direct2D por software (WARP): uno por
hardware carga en el proceso los controladores de cada GPU (130 MB en un
portátil con dos) y no se descargan nunca. Un fotograma de la selección tarda
unos 15 ms sobre 3840×1080; aparece unos 100 ms después de la orden. Tras
capturar, los hilos de trabajo que Windows abre para dibujar se retiran solos
en menos de un minuto.

## Compilar

Rust con el objetivo `x86_64-pc-windows-gnullvm` y el enlazador de
[llvm-mingw](https://github.com/mstorsjo/llvm-mingw) — sin Visual Studio.

1. Instalar [rustup](https://rustup.rs). `rust-toolchain.toml` elige la
   toolchain `stable-x86_64-pc-windows-gnullvm`.
2. Descomprimir una versión de llvm-mingw (`*-ucrt-x86_64.zip`) en
   `%LOCALAPPDATA%\Programs\llvm-mingw`, o apuntar `LLVM_MINGW` a donde esté.
3. Compilar y probar a través del script, que pone llvm-mingw en el PATH:

```powershell
./scripts/cargo.ps1 test
./scripts/cargo.ps1 build --release
```

El instalador necesita además [WiX 5.0.2](https://wixtoolset.org)
(`dotnet tool install --global wix --version 5.0.2`; las versiones 6 y 7
exigen una cuota de mantenimiento):

```powershell
./installer/build.ps1
```

Deja `dist/ScreenShotCM-<versión>.msi` y su SHA-256.

## Microsoft Store (MSIX)

`msix/build.ps1` genera `dist/ScreenShotCM-<versión>.msix` para enviarlo a
la Store. Necesita `makeappx.exe` del paquete
[Microsoft.Windows.SDK.BuildTools](https://www.nuget.org/packages/Microsoft.Windows.SDK.BuildTools)
descomprimido en `%LOCALAPPDATA%\Programs\windows-sdk-buildtools` (o la
ruta en `SDK_BUILDTOOLS`).

```powershell
./msix/build.ps1 -IdentityName "<Package/Identity/Name>" `
                 -Publisher "<Package/Identity/Publisher>" `
                 -PublisherDisplayName "<Nombre del editor>"
```

Los tres valores salen de Partner Center → la aplicación → *Product
management → Product identity*. Sin ellos el paquete lleva valores de
prueba: sirve para comprobar que se genera, no para enviarlo. Se sube sin
firmar: la Store lo firma.

Dentro del paquete, *Iniciar con Windows* usa la tarea de inicio declarada
en `msix/AppxManifest.xml` y el enlace `screenshotcm://` también se declara
allí. La aplicación y el paquete están en español e inglés.

## Idioma

Español e inglés. Por defecto, el de Windows; se puede fijar en *Ajustes →
General → Idioma*. Cada texto de la interfaz está escrito en el código como
`tr("español", "English")`.

## Uso desde la línea de comandos o una URL

Un segundo arranque entrega su orden al agente que ya corre:

```
screenshotcm.exe --capture-area
screenshotcm.exe --pin C:\captura.png
screenshotcm.exe C:\imagen.png          (la abre en el editor)
screenshotcm://capture-window
```

Verbos: `capture-area`, `capture-previous-area`, `capture-fullscreen`,
`capture-window`, `self-timer`, `open-from-clipboard`, `open-file`,
`open-annotate`, `pin`, `open-history`, `restore-recently-closed`,
`unlock-pins`, `close-pins`, `open-save-folder`, `open-settings`,
`reload-settings`, `quit`.

## Ajustes y datos

Menú de la bandeja → *Ajustes…*. Se guardan en
`%APPDATA%\ScreenShotCM\settings.ini`; el historial y los registros, en
`%LOCALAPPDATA%\ScreenShotCM`.

## Dependencias

Una sola: [`windows`](https://crates.io/crates/windows) (MIT OR Apache-2.0).
La bandeja, los atajos, el portapapeles, la codificación de imágenes (WIC) y
el dibujo (Direct2D) se usan directamente desde Windows. Ver
[THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md).
