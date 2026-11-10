# menu: a template for screens with buttons

Copy this folder and change the screens. The ideas:

- the screen the person is on is one number, `app.screen` (0 menu, 1 play, 2 options, 3 about)
- `lib/layout.lua` builds the buttons of each screen once; the system (sys/screens.lua)
  tests them for clicks, the drawing (draw/screens.lua) draws the same buttons. Build them
  the same way in both, so what is drawn is what is clickable
- `lib.ui` is built into Lust: `local ui = ecs.call('lib.ui')`. A button knows its width
  and height from the text; `ui.menu` stacks buttons; `ui.update_menu` moves the choice
  with arrows, mouse and enter; `ui.clicked(b)` is a click; `ui.label` and `ui.paragraph`
  place text
- saves: `ecs.save('auto')` at each change of screen, `ecs.load('auto')` at the first tick.
  Only the data is saved (not the code), in the folder of your data (on Windows
  `%APPDATA%\lust\menu`), so the score and the options stay when the game starts again
