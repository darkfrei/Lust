-- the launcher: the buttons of the games on this page, for the system and the drawing.
-- gives {list, first, total, page, pages, names}; sel is the number of the chosen game
local PAGE = 8
return function(sel)
  local ui = ecs.call('lib.ui')
  local names = ecs.with('title')
  local total = #names
  local page = (math.max(1, sel) - 1) // PAGE
  local first = page * PAGE + 1
  local items = {}
  for i = first, math.min(total, first + PAGE - 1) do
    items[#items + 1] = ecs.get(names[i], 'title')
  end
  local list = ui.menu(items, 400, 150, { size = 28, gap = 10, w = 460 })
  return { list = list, first = first, total = total, page = page + 1, pages = (total + PAGE - 1) // PAGE, names = names }
end
