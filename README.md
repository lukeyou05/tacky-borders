# tacky-borders

![image](https://github.com/user-attachments/assets/e1786c07-4168-42ca-8ada-ccbabcf74a63)
tacky-borders lets you customize the borders on Windows 11 (and maybe 10?).

## Installation:
Download your desired version from the releases page, unzip it, and run the .exe! tacky-borders will automatically generate a config file for you in ```%userprofile%/.config/tacky-borders/```.

Alternatively, if you want to build it yourself, first make sure you have installed the required tools such as rustup, cargo, and MSVC build tools. Then, just clone the repo, cd into tacky-borders, and do ```cargo build``` or ```cargo run```

## Uninstallation
Just delete the .exe and the config file located in ```%userprofile%/.config/tacky-borders/```.

:(

## Configuration Options:
The config.yaml is located in ```%userprofile%/.config/tacky-borders/```. You can easily access this folder by right clicking on the tray icon and hitting "Show Config"

The following options are customizable and are included in the auto-generated config file:
- border_width: Thickness of the borders
- border_offset: How close the borders are to the window edges
- border_radius: Leave it at -1 to let tacky-borders handle the radius adjstments, or set it to any other value to use as the radius.
- active_color: Color of the active window. Currently, you can use "accent" to grab the Windows accent color, or use your own hex code like "#ffffff"
- inactive_color: Color of the inactive window. Again, you can use "accent" to grab the Windows accent color, or use your own hex code like "#ffffff"

Additionally, there are some optional config options that are not included in the auto-generated config file:
- init_delay: The delay in milliseconds between when a new window is first opened and when the border shows itself. I recommend setting this to 0 if you have disabled Windows animations.
- unminimize_delay: The delay in milliseconds between when a window is restored/unminimized and when the border shows itself. I also recommend setting this to 0 if you have disabled Windows animations.

Unfortunately, these delays are necessary due to limitations with the Win32 API regarding window animations.

## Comparison to cute-borders
Here is another great app that achieves similar fuctionality: https://github.com/keifufu/cute-borders. I've taken a lot of inspiration from them and would highly recommend checking them out! Our apps have totally different implementations, each with their own limitations, but which one you should use boils down to the following:

I recommend using tacky-borders if you want:
- borders thicker than 1px
- gradients (upcoming feature)
- animations (also upcoming lol)
- Windows 10 support (not fully tested)

Otherwise, I recommend using cute-borders because I find it to be more stable and performant.
