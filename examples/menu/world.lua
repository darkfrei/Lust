-- a template: a title screen with a menu, a game screen, options, an about page.
-- the screens are numbers: 0 menu, 1 play, 2 options, 3 about. the score and the options
-- are saved in the slot auto at each change of screen and loaded at the start
return {
  types = { screen = 'num', sel = 'num', score = 'num', fast = 'num', loaded = 'num' },
  entities = {
    app = { screen = 0, sel = 1, score = 0, fast = 0, loaded = 0 },
  },
}
