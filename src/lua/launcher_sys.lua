-- the launcher: arrows and enter, or the mouse; a chosen game is started by the program
return function(dt)
  local ui = ecs.call('lib.ui')
  local sel = ecs.get('menu', 'sel')
  local p = ecs.call('lib.pick', sel)
  if p.total == 0 then
    return
  end
  local mine, chosen, edge = ui.update_menu(p.list, sel - p.first + 1, { wrap = false })
  sel = math.max(1, math.min(p.total, p.first + mine - 1 + edge))
  ecs.set('menu', 'sel', sel)
  if chosen then
    ecs.launch(p.names[p.first + chosen - 1])
  end
end
