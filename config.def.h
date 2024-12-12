/* See LICENSE file for copyright and license details. */
/* Default settings; can be overriden by command line. */
#include "enums.h"

#ifndef INSTANTMENU_CONFIG_H
#define INSTANTMENU_CONFIG_H

static int topbar = 1;                      /* -b  option; if 0, instantmenu appears at bottom     */
static int centered = 0;                    /* -c option; centers dmenu on screen */
static int followcursor = 0;                    /* -c option; centers dmenu on screen */
static int min_width = 500;                    /* minimum width when centered */

static int instant = 0;
static int spaceconfirm = 0;
static int fuzzy = 1;
static int prematch = 0;
static int smartcase = 0;
static int exact = 0;
static int sely = 0;
static int animated = 0;
static int framecount = 7;
static int fullheight = 0;
static unsigned int lineheight = 0;         /* -h option; minimum height of a menu line     */

/* -fn option overrides fonts[0]; default X11 font or font set */
static const char *fonts[] = {
	"Inter-Regular:size=12", 
	"Fira Code Nerd Font:size=14", 
	"JoyPixels:pixelsize=20:antialias=true:autohint=true",
};

static const char *prompt      = NULL;      /* -p  option; prompt to the left of input field */
static const char *searchtext      = NULL;      /* -p  option; prompt to the left of input field */
static const char *leftcmd      = NULL;      /* -p  option; prompt to the left of input field */
static const char *rightcmd      = NULL;      /* -p  option; prompt to the left of input field */
static const char *colors[SchemeLast][9] = {
	/*     fg         bg     darker      */
	[SchemeNorm] = { "#DFDFDF", "#121212", "#3E485B" },
	[SchemeFade] = { "#575E70", "#121212", "#3E485B" },
	[SchemeHighlight] = { "#DFDFDF", "#384252", "#272727" },
	[SchemeHover] = { "#DFDFDF", "#272727", "#2E2E2E" },
	[SchemeSel] = { "#000000", "#8AB4F8", "#536DFE" },
	[SchemeOut] = { "#000000", "#3579CA", "#3579CA" },
	[SchemeGreen] = { "#000000", "#81c995", "#1e8e3e" },
	[SchemeRed] = { "#000000", "#f28b82", "#d93025" },
	[SchemeYellow] = { "#000000", "#fdd663", "#f9ab00" },
};

/* -l option; if nonzero, instantmenu uses vertical list with given number of lines */
/* -g option; controls columns in grid if nonzero and lines is nonzero */
static unsigned int lines      = 0;
static unsigned int columns    = 1;

/*
 * Characters not considered part of a word while deleting words
 * for example: " /?\"&[]"
 */
static const char worddelimiters[] = " ";

/* -ps option; preselected item starting from 0 */
static unsigned int preselected = 0;

/* Size of the window border */
static unsigned int border_width = 0;

#endif /* INSTANTMENU_CONFIG_H */