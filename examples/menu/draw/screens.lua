-- what the screens look like
return function()
  local ui = ecs.call('lib.ui')
  local layout = ecs.call('lib.layout')
  local screen = ecs.get('app', 'screen')
  gfx.size(800, 600)
  gfx.clear(ui.theme.back)
  if screen == 0 then
    ui.label('My Game', 400, 90, { size = 64, align = 'center' })
    ui.draw_menu(layout.menu(), ecs.get('app', 'sel'))
    ui.label('arrows and enter, or the mouse', 400, 540, { size = 18, align = 'center', color = ui.theme.dim })
    return
  end
  if screen == 1 then
    local b = layout.play()
    ui.draw(b.back)
    ui.draw(b.hit)
    ui.label('Score: ' .. string.format('%d', ecs.get('app', 'score')), 400, 160, { size = 40, align = 'center' })
    ui.label('click the button, or press space. escape goes back', 400, 540, { size = 18, align = 'center', color = ui.theme.dim })
  elseif screen == 2 then
    local b = layout.options()
    ui.draw(b.back)
    ui.draw(b.fast)
    ui.label('Options', 400, 90, { size = 48, align = 'center' })
    ui.label('fast mode gives 5 points a click', 400, 320, { size = 18, align = 'center', color = ui.theme.dim })
  elseif screen == 3 then
    ui.draw(layout.about().back)
    ui.label('About', 400, 90, { size = 48, align = 'center' })
    ui.paragraph('This is a template for screens with buttons, made with Lust. Copy the folder, change the screens, and ask your AI to build your game on top of it.', 150, 200, 500, { size = 24 })
  end
end
