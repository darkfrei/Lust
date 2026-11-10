-- what the buttons do. the screens: 0 menu, 1 play, 2 options, 3 about
local MENU, PLAY, OPTIONS, ABOUT = 0, 1, 2, 3

-- a change of screen also saves (after this tick): the score and the options stay
local function go(screen)
  ecs.set('app', 'screen', screen)
  ecs.set('app', 'sel', 1)
  ecs.save('auto')
end

return function(dt)
  -- the first tick: the last save back, if there is one (ecs.load gives false if not)
  if ecs.get('app', 'loaded') == 0 then
    ecs.set('app', 'loaded', 1)
    ecs.load('auto')
  end
  local ui = ecs.call('lib.ui')
  local layout = ecs.call('lib.layout')
  local screen = ecs.get('app', 'screen')
  if screen == MENU then
    local sel, chosen = ui.update_menu(layout.menu(), ecs.get('app', 'sel'))
    ecs.set('app', 'sel', sel)
    -- the buttons are in the order of the screens: Play, Options, About
    if chosen then go(chosen) end
    return
  end
  if input.pressed('escape') then
    go(MENU)
    return
  end
  if screen == PLAY then
    local b = layout.play()
    if ui.clicked(b.back) then go(MENU) end
    if ui.clicked(b.hit) or input.pressed('space') then
      local step = ecs.get('app', 'fast') == 1 and 5 or 1
      ecs.set('app', 'score', ecs.get('app', 'score') + step)
    end
  elseif screen == OPTIONS then
    local b = layout.options()
    if ui.clicked(b.back) then go(MENU) end
    if ui.clicked(b.fast) then ecs.set('app', 'fast', 1 - ecs.get('app', 'fast')) end
  elseif screen == ABOUT then
    if ui.clicked(layout.about().back) then go(MENU) end
  end
end
