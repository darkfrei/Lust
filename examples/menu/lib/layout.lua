-- the buttons of every screen, built the same way for the tick and for the drawing
local L = {}

function L.menu()
  local ui = ecs.call('lib.ui')
  return ui.menu({ 'Play', 'Options', 'About' }, 400, 220, { size = 32, w = 300, gap = 16 })
end

function L.play()
  local ui = ecs.call('lib.ui')
  return {
    back = ui.button('Menu', 20, 20, { size = 20 }),
    hit = ui.button('Click me', 300, 260, { size = 36, w = 200, h = 80 }),
  }
end

function L.options()
  local ui = ecs.call('lib.ui')
  local fast = ecs.get('app', 'fast') == 1
  return {
    back = ui.button('Menu', 20, 20, { size = 20 }),
    fast = ui.button(fast and 'Fast mode: on' or 'Fast mode: off', 250, 240, { size = 28, w = 300 }),
  }
end

function L.about()
  local ui = ecs.call('lib.ui')
  return { back = ui.button('Menu', 20, 20, { size = 20 }) }
end

return function()
  return L
end
