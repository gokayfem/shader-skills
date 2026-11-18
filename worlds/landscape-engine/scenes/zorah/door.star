# A door's leaf: someone near the doorway uses it, and it swings open or shut about its hinge.
# A part of a door that cannot swing with either leaf slides out of the way instead.
#
# Near is measured from wherever the game is watched from, so it works flying a camera through
# a scene that has no floor to walk on, as it does for a player on foot.

def on_begin(self):
    self.vars.open = False
    self.vars.shut_yaw = self.yaw
    self.vars.shut_at = self.position
    self.vars.turned = 0.0      # from 0, shut, to 1, open

def on_tick(self, dt):
    if pressed("interact"):
        off = viewer() - self.middle
        if sqrt(off.x * off.x + off.z * off.z) <= self.reach:
            self.vars.open = not self.vars.open

    turned = self.vars.turned
    step = dt / self.seconds
    if self.vars.open:
        turned = min(1.0, turned + step)
    else:
        turned = max(0.0, turned - step)
    if turned == self.vars.turned:
        return
    self.vars.turned = turned

    # Slow to start and slow to stop, as a heavy door is.
    eased = turned * turned * (3.0 - 2.0 * turned)
    self.yaw = self.vars.shut_yaw + eased * self.swing * 3.14159265 / 180.0
    self.position = self.vars.shut_at + eased * self.slide
