return function()
  gfx.clear('#000')
  for y = 10, 590, 40 do gfx.rect(398, y, 4, 20, '#444') end
  for _, p in ipairs({ 'left', 'right' }) do
    local h = ecs.get(p, 'h')
    gfx.rect(ecs.get(p, 'x') - 5, ecs.get(p, 'y') - h / 2, 10, h, '#fff')
  end
  gfx.circle(ecs.get('ball', 'x'), ecs.get('ball', 'y'), 8, '#ff0')
  gfx.text(330, 20, 40, '#fff', ecs.get('left', 'score'))
  gfx.text(445, 20, 40, '#fff', ecs.get('right', 'score'))
end
