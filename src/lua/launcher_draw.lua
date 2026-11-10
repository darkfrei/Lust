-- the launcher: the title, the games as buttons, a hint
return function()
  local ui = ecs.call('lib.ui')
  gfx.size(800, 600)
  gfx.clear(ui.theme.back)
  ui.label('Lust', 400, 36, { size = 64, align = 'center' })
  local sel = ecs.get('menu', 'sel')
  local p = ecs.call('lib.pick', sel)
  if p.total == 0 then
    ui.paragraph(ecs.get('menu', 'hint'), 80, 180, 640, { size = 24 })
    return
  end
  ui.label('Choose a game: arrows and enter, or the mouse', 400, 110, { size = 18, align = 'center', color = ui.theme.dim })
  ui.draw_menu(p.list, sel - p.first + 1)
  if p.pages > 1 then
    ui.label('page ' .. p.page .. ' of ' .. p.pages, 400, 520, { size = 18, align = 'center', color = ui.theme.dim })
  end
  ui.label('F1 in a game brings you back here', 400, 560, { size = 16, align = 'center', color = ui.theme.dim })
end
