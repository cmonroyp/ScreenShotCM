# Seguridad · Security

## Reportar una vulnerabilidad

No abras un issue público. Usa **Security → Report a vulnerability** en este
repositorio (aviso privado de GitHub): solo el autor lo ve hasta que haya una
corrección publicada. Incluye la versión, los pasos para reproducirlo y qué
consigue un atacante.

## Reporting a vulnerability

Please do not open a public issue. Use **Security → Report a vulnerability** in
this repository (GitHub private reporting): only the author sees it until a fix
is released. Include the version, the steps to reproduce it and what an
attacker gains.

## Versiones con soporte · Supported versions

Solo la última versión publicada · Only the latest release.

## Superficie · Surface

- ScreenShotCM no abre conexiones de red ni escucha en ningún puerto.
  ScreenShotCM opens no network connections and listens on no port.
- El enlace `screenshotcm://` solo acepta órdenes conocidas y rutas de archivo
  en una unidad local; una ruta de red se rechaza.
  The `screenshotcm://` link accepts only known verbs and file paths on a
  local drive; a network path is refused.
- Se ejecuta con los permisos del usuario, sin administrador.
  It runs with the user's permissions, without elevation.
