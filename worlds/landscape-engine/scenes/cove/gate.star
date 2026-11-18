# The fort's gate: someone near it uses it, and it rises or falls.

def on_begin(self):
    self.vars.open = False
    self.vars.closed_y = self.position.y

def on_interact(self, at):
    offset = at - self.position
    if sqrt(offset.x * offset.x + offset.z * offset.z) > self.reach:
        return
    if self.locked:
        log("The gate is locked.")
        return
    self.vars.open = not self.vars.open
    log("The gate rises." if self.vars.open else "The gate falls.")

def on_tick(self, dt):
    target = self.vars.closed_y + (self.lift if self.vars.open else 0.0)
    y = self.position.y
    step = self.speed * dt
    if abs(target - y) <= step:
        y = target
    elif target > y:
        y += step
    else:
        y -= step
    if y != self.position.y:
        self.position = vec3(self.position.x, y, self.position.z)
