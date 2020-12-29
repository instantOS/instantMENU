/* See LICENSE file for copyright and license details. */
#include <ctype.h>
#include <locale.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <time.h>
#include <unistd.h>

#include <X11/Xlib.h>
#include <X11/Xatom.h>
#include <X11/Xutil.h>
#ifdef XINERAMA
#include <X11/extensions/Xinerama.h>
#endif
#include <X11/Xft/Xft.h>
#include <X11/cursorfont.h>

#include "drw.h"
#include "util.h"

/* macros */
#define INTERSECT(x,y,w,h,r)  (MAX(0, MIN((x)+(w),(r).x_org+(r).width)  - MAX((x),(r).x_org)) \
                             * MAX(0, MIN((y)+(h),(r).y_org+(r).height) - MAX((y),(r).y_org)))
#define LENGTH(X)             (sizeof X / sizeof X[0])
#define TEXTW(X)              (commented ? bh : drw_fontset_getwidth(drw, (X)) + lrpad)

#define NUMBERSMAXDIGITS      100
#define NUMBERSBUFSIZE        (NUMBERSMAXDIGITS * 2) + 1
#define BUTTONMASK              (ButtonPressMask|ButtonReleaseMask)
#define MOUSEMASK               (BUTTONMASK|PointerMotionMask)


/* enums */
enum { SchemeNorm, SchemeFade, SchemeHighlight, SchemeHover, SchemeSel, SchemeOut, SchemeGreen, SchemeYellow, SchemeRed, SchemeLast }; /* color schemes */

struct item {
	char *text;
	struct item *left, *right;
	int out;
	double distance;
};

static char numbers[NUMBERSBUFSIZE] = "";
static int tempnumer;
static char text[BUFSIZ] = "";
static char *embed;
static int bh, mw, mh;
static int dmx = 0, dmy = 0; /* put instantmenu at these x and y offsets */
static int dmw = 0; /* make instantmenu this wide */
static int rightxoffset = 0; /* make instantmenu x offset come from the right */
static int inputw = 0, promptw, toast = 0, inputonly = 0, passwd = 0, nograb = 0, alttab = 0, tabbed = 0;
static int lrpad; /* sum of left and right padding */
static size_t cursor;
static struct item *items = NULL;
static struct item *matches, *matchend;
static struct item *prev, *curr, *next, *sel;
static int mon = -1, screen;
static int managed = 0;
static int commented = 0;

static Atom clip, utf8;
static Display *dpy;
static Window root, parentwin, win;
static XIC xic;

static Drw *drw;
static Clr *scheme[SchemeLast];

#include "config.h"

static int (*fstrncmp)(const char *, const char *, size_t) = strncmp;
static char *(*fstrstr)(const char *, const char *) = strstr;

int
getrootptr(int *x, int *y)
{
	int di;
	unsigned int dui;
	Window dummy;

	return XQueryPointer(dpy, root, &dummy, &dummy, x, y, &di, &di, &dui);
}

static void
appenditem(struct item *item, struct item **list, struct item **last)
{
	if (*last)
		(*last)->right = item;
	else
		*list = item;

	item->left = *last;
	item->right = NULL;
	*last = item;
}

static void
calcoffsets(void)
{
	int i, n;

	if (lines > 0)
		n = lines * bh;
	else
		n = mw - (promptw + inputw + TEXTW("<") + TEXTW(">"));
	/* calculate which items will begin the next page and previous page */
	for (i = 0, next = curr; next; next = next->right)
		if ((i += (lines > 0) ? bh : MIN(TEXTW(next->text), n)) > n)
			break;
	for (i = 0, prev = curr; prev && prev->left; prev = prev->left)
		if ((i += (lines > 0) ? bh : MIN(TEXTW(prev->left->text), n)) > n)
			break;
}

static int
max_textw(void)
{
	int len = 0;
	for (struct item *item = items; item && item->text; item++)
		len = MAX(TEXTW(item->text), len);
	return len;
}


static void
cleanup(void)
{
	size_t i;

	XUngrabKey(dpy, AnyKey, AnyModifier, root);
	for (i = 0; i < SchemeLast; i++)
		free(scheme[i]);
	drw_free(drw);
	XSync(dpy, False);
	XCloseDisplay(dpy);
}

static char *
cistrstr(const char *s, const char *sub)
{
	size_t len;

	for (len = strlen(sub); *s; s++)
		if (!strncasecmp(s, sub, len))
			return (char *)s;
	return NULL;
}

static int
drawitem(struct item *item, int x, int y, int w)
{
	int iscomment = 0;
	if (item->text[0] == '>') {
		if (item->text[1] == '>') {
			iscomment = 3;
			switch (item->text[2])
			{
				case 'r':
					drw_setscheme(drw, scheme[SchemeRed]);
					break;
				case 'g':
					drw_setscheme(drw, scheme[SchemeGreen]);
					break;
				case 'y':
					drw_setscheme(drw, scheme[SchemeYellow]);
					break;
				case 'h':
					drw_setscheme(drw, scheme[SchemeHighlight]);
					break;

				case 'b':
					drw_setscheme(drw, scheme[SchemeSel]);
					break;
				default:
					iscomment = 1;
					drw_setscheme(drw, scheme[SchemeNorm]);
					break;
			}
		} else {
			drw_setscheme(drw, scheme[SchemeNorm]);
			iscomment = 1;
		}

	} else if (item->text[0] == ':') {
		iscomment = 2;
		if (item == sel) {
			switch (item->text[1])
			{
			case 'r':
				drw_setscheme(drw, scheme[SchemeRed]);
				break;
			case 'g':
				drw_setscheme(drw, scheme[SchemeGreen]);
				break;
			case 'y':
				drw_setscheme(drw, scheme[SchemeYellow]);
				break;
			case 'b':
				drw_setscheme(drw, scheme[SchemeSel]);
				break;
			default:
				drw_setscheme(drw, scheme[SchemeSel]);
				iscomment = 0;
				break;
			}
		} else {
			drw_setscheme(drw, scheme[SchemeNorm]);
		}
	} else {
		if (item == sel)
			drw_setscheme(drw, scheme[SchemeSel]);
		else if (item->out)
			drw_setscheme(drw, scheme[SchemeOut]);
		else
			drw_setscheme(drw, scheme[SchemeNorm]);
	}

	int temppadding;
	temppadding = 0;
	if (iscomment == 2) {
		if (item->text[2] == ' ') {
			temppadding = drw->fonts->h * 3;
			animated = 1;
			char dest[1000];
			strcpy(dest, item->text);
			dest[6] = '\0';
			drw_text(drw, x, y, temppadding, lineheight, temppadding/2.6, dest  + 3, 0, item == sel);
			iscomment = 6;
			drw_setscheme(drw, sel == item ? scheme[SchemeHover] : scheme[SchemeNorm]);
		}
	}

	char *output;
	if (commented) {
		static char onestr[2];
		onestr[0] = item->text[0];
		onestr[1] = '\0';
		output = onestr;
	} else {
		output = item->text;
	}

	if (item == sel)
		sely = y;
	return drw_text(drw, x + ((iscomment == 6) ? temppadding : 0), y, commented ? bh : (w - ((iscomment == 6) ? temppadding : 0)), bh,
		commented ? (bh - drw_fontset_getwidth(drw, (output))) / 2: lrpad / 2, output + iscomment, 0, (iscomment == 3 || item == sel));
}

static void
recalculatenumbers()
{
	unsigned int numer = 0, denom = 0;
	struct item *item;
	if (matchend) {
		numer++;
		for (item = matchend; item && item->left; item = item->left)
			numer++;
	}

	if (toast) {
		tempnumer = 0;
		return;
	}

	for (item = items; item && item->text; item++)
		denom++;
	if (numer > 1) {
		if (lines > 1) {
			if (numer > lines)
				tempnumer = 1;
			else
				tempnumer = 0;
		} else {
			tempnumer = 1;
		}
	} else {
		tempnumer = 0;
	}
	snprintf(numbers, NUMBERSBUFSIZE, "%d/%d", numer, denom);
}


static void
drawmenu(void)
{
	unsigned int curpos;
	struct item *item;
	int x = 0, y = 0, fh = drw->fonts->h, w;

	char *censort;
	drw_setscheme(drw, scheme[SchemeNorm]);
	drw_rect(drw, 0, 0, mw, mh, 1, 1, 0);
	if (commented && matches)
		prompt = sel->text + 1;

	if (prompt && *prompt) {
		drw_setscheme(drw, scheme[SchemeSel]);
		if (lines < 8) {
			x = drw_text(drw, x, 0, promptw, bh * (lines + 1), lrpad / 2, prompt, 0, 1);
		} else {
			x = drw_text(drw, x, 0, promptw, bh, lrpad / 2, prompt, 0, 1);
		}
	}

	/* draw input field */
	w = (lines > 0 || !matches) ? mw - x : inputw;
	drw_setscheme(drw, scheme[SchemeNorm]);
	
	if (passwd) {
			censort = ecalloc(1, sizeof(text));
			memset(censort, '.', strlen(text));
			drw_text(drw, x, 0, w, bh, lrpad / 2, censort, 0, 0);
			free(censort);
	} else {
		if (text[0] != '\0') {
			drw_text(drw, x, 0, w, bh, lrpad / 2, text, 0, 0);
		} else  if (searchtext){
			drw_setscheme(drw, scheme[SchemeFade]);
			drw_text(drw, x, 0, w, bh, lrpad / 2, searchtext, 0, 0);
			drw_setscheme(drw, scheme[SchemeNorm]);
		}
	}
	curpos = TEXTW(text) - TEXTW(&text[cursor]);
	if ((curpos += lrpad / 2 - 1) < w) {
		drw_setscheme(drw, scheme[SchemeNorm]);
		// disable cursor on password prompt
		if (!passwd)
			drw_rect(drw, x + curpos, 2 + (bh-fh)/2, 2, fh - 4, 1, 0, 0);

	}

	recalculatenumbers();

	if (lines > 0) {
		/* draw vertical list */
		for (item = curr; item != next; item = item->right)
			drawitem(item, x, y += bh, mw - x);
	} else if (matches) {
		/* draw horizontal list */
		x += inputw;
		w = TEXTW("<");
		if (curr->left) {
			drw_setscheme(drw, scheme[SchemeNorm]);
			drw_text(drw, x, 0, w, bh, lrpad / 2, "<", 0, 0);
		}
		x += w;
		for (item = curr; item != next; item = item->right)
			x = drawitem(item, x, 0, MIN(TEXTW(item->text), mw - x - TEXTW(">") - TEXTW(numbers)));

		if (next) {
			w = TEXTW(">");
			drw_setscheme(drw, scheme[SchemeNorm]);
			if (tempnumer)
				drw_text(drw, mw - w - TEXTW(numbers), 0, w, bh, lrpad / 2, ">", 0, 0);

		}
	}


	drw_setscheme(drw, scheme[SchemeNorm]);
	if (tempnumer)
		drw_text(drw, mw - TEXTW(numbers), 0, TEXTW(numbers), bh, lrpad / 2, numbers, 0, 0);
	drw_map(drw, win, 0, 0, mw, mh);
}

static void
grabfocus(void)
{
	if (toast)
		return;

	struct timespec ts = { .tv_sec = 0, .tv_nsec = 10000000  };
	Window focuswin;
	int i, revertwin;

	for (i = 0; i < 100; ++i) {
		XGetInputFocus(dpy, &focuswin, &revertwin);
		if (focuswin == win)
			return;
		if (managed) {
			XTextProperty prop;
			char *windowtitle = prompt != NULL ? prompt : "dmenu";
			Xutf8TextListToTextProperty(dpy, &windowtitle, 1, XUTF8StringStyle, &prop);
			XSetWMName(dpy, win, &prop);
			XSetTextProperty(dpy, win, &prop, XInternAtom(dpy, "_NET_WM_NAME", False));
			XFree(prop.value);
		} else {
			XSetInputFocus(dpy, win, RevertToParent, CurrentTime);
		}
		nanosleep(&ts, NULL);
	}
	die("cannot grab focus");
}

static void
grabkeyboard(void)
{
	if (toast)
		return;
	struct timespec ts = { .tv_sec = 0, .tv_nsec = 1000000  };
	int i;

	if (nograb)
		return;

	if (embed || managed)
		return;
	/* try to grab keyboard, we may have to wait for another process to ungrab */
	for (i = 0; i < 1000; i++) {
		if (XGrabKeyboard(dpy, DefaultRootWindow(dpy), True, GrabModeAsync,
		                  GrabModeAsync, CurrentTime) == GrabSuccess)
			return;
		nanosleep(&ts, NULL);
	}
	die("cannot grab keyboard");
}

static void
grabpointer(void)
{
	XGrabPointer(dpy, root, False, MOUSEMASK, GrabModeAsync, GrabModeAsync,
		None, XCreateFontCursor(dpy, XC_fleur), CurrentTime);
}

int
compare_distance(const void *a, const void *b)
{
	struct item *da = *(struct item **) a;
	struct item *db = *(struct item **) b;

	if (!db)
		return 1;
	if (!da)
		return -1;

	return da->distance == db->distance ? 0 : da->distance < db->distance ? -1 : 1;
}

void
fuzzymatch(void)
{
	/* bang - we have so much memory */
	struct item *it;
	struct item **fuzzymatches = NULL;
	char c;
	int number_of_matches = 0, i, pidx, sidx, eidx;
	int text_len = strlen(text), itext_len;

	matches = matchend = NULL;

	/* walk through all items */
	for (it = items; it && it->text; it++) {
		if (text_len) {
			itext_len = strlen(it->text);
			pidx = 0; /* pointer */
			sidx = eidx = -1; /* start of match, end of match */
			/* walk through item text */
			for (i = 0; i < itext_len && (c = it->text[i]); i++) {
				/* fuzzy match pattern */
				if (!fstrncmp(&text[pidx], &c, 1)) {
					if(sidx == -1)
						sidx = i;
					pidx++;
					if (pidx == text_len) {
						eidx = i;
						break;
					}
				}
			}
			/* build list of matches */
			if (eidx != -1) {
				/* compute distance */
				/* add penalty if match starts late (log(sidx+2))
				 * add penalty for long a match without many matching characters */
				it->distance = log(sidx + 2) + (double)(eidx - sidx - text_len);
				/* fprintf(stderr, "distance %s %f\n", it->text, it->distance); */
				appenditem(it, &matches, &matchend);
				number_of_matches++;
			}
		} else {
			appenditem(it, &matches, &matchend);
		}
	}

	if (number_of_matches) {
		/* initialize array with matches */
		if (!(fuzzymatches = realloc(fuzzymatches, number_of_matches * sizeof(struct item*))))
			die("cannot realloc %u bytes:", number_of_matches * sizeof(struct item*));
		for (i = 0, it = matches; it && i < number_of_matches; i++, it = it->right) {
			fuzzymatches[i] = it;
		}
		/* sort matches according to distance */
		qsort(fuzzymatches, number_of_matches, sizeof(struct item*), compare_distance);
		/* rebuild list of matches */
		matches = matchend = NULL;
		for (i = 0, it = fuzzymatches[i];  i < number_of_matches && it && \
				it->text; i++, it = fuzzymatches[i]) {
			appenditem(it, &matches, &matchend);
		}
		free(fuzzymatches);
	}
	curr = sel = matches;
	
	if(instant && matches && matches==matchend) {
		puts(matches->text);
		cleanup();
		exit(0);
	}

	calcoffsets();
}

static void
match(void)
{

	if (commented) {
		struct item *it;
		for (it = items; it && it->text; it++) {
			if (text && it->text[0] == text[0]) {
				puts(it->text);
				cleanup();
				exit(0);
			}	
		}
		// exit if no match is found
		if (text[0] != '\0') {
			cleanup();
			exit(0);
		}
	}

	if (fuzzy) {
		fuzzymatch();
		return;
	}
	static char **tokv = NULL;
	static int tokn = 0;

	char buf[sizeof text], *s;
	int i, tokc = 0;
	size_t len, textsize;
	struct item *item, *lprefix, *lsubstr, *prefixend, *substrend;

	strcpy(buf, text);
	/* separate input text into tokens to be matched individually */
	for (s = strtok(buf, " "); s; tokv[tokc - 1] = s, s = strtok(NULL, " "))
		if (++tokc > tokn && !(tokv = realloc(tokv, ++tokn * sizeof *tokv)))
			die("cannot realloc %u bytes:", tokn * sizeof *tokv);
	len = tokc ? strlen(tokv[0]) : 0;

	matches = lprefix = lsubstr = matchend = prefixend = substrend = NULL;
	textsize = strlen(text) + 1;
	for (item = items; item && item->text; item++) {
		for (i = 0; i < tokc; i++)
			if (!fstrstr(item->text, tokv[i]))
				break;
		if (i != tokc) /* not all tokens match */
			continue;
		/* exact matches go first, then prefixes, then substrings */
		if (!tokc || !fstrncmp(text, item->text, textsize))
			appenditem(item, &matches, &matchend);
		else if (!fstrncmp(tokv[0], item->text, len))
			appenditem(item, &lprefix, &prefixend);
		else if (!exact)
			appenditem(item, &lsubstr, &substrend);
	}
	if (lprefix) {
		if (matches) {
			matchend->right = lprefix;
			lprefix->left = matchend;
		} else
			matches = lprefix;
		matchend = prefixend;
	}
	if (lsubstr) {
		if (matches) {
			matchend->right = lsubstr;
			lsubstr->left = matchend;
		} else
			matches = lsubstr;
		matchend = substrend;
	}
	curr = sel = matches;

	if(instant && matches && matches==matchend && !lsubstr) {
		puts(matches->text);
		cleanup();
		exit(0);
	}

	calcoffsets();
}

static void
insert(const char *str, ssize_t n)
{
	if (strlen(text) + n > sizeof text - 1)
		return;
	/* move existing text out of the way, insert new text, and update cursor */
	memmove(&text[cursor + n], &text[cursor], sizeof text - cursor - MAX(n, 0));
    if (n > 0) {
		memcpy(&text[cursor], str, n);
        int i;
        if (smartcase) {
            for (i = 0; i < strlen(text); i++) {
                if (text[i] >= 65 && text[i] <= 90) {
                    smartcase = 0;
                    fstrncmp = strncmp;
                    fstrstr = strstr;
                }
            }
        }
    }
	cursor += n;
	match();
}

static size_t
nextrune(int inc)
{
	ssize_t n;

	/* return location of next utf8 rune in the given direction (+1 or -1) */
	for (n = cursor + inc; n + inc >= 0 && (text[n] & 0xc0) == 0x80; n += inc)
		;
	return n;
}

static void
movewordedge(int dir)
{
	if (dir < 0) { /* move cursor to the start of the word*/
		while (cursor > 0 && strchr(worddelimiters, text[nextrune(-1)]))
			cursor = nextrune(-1);
		while (cursor > 0 && !strchr(worddelimiters, text[nextrune(-1)]))
			cursor = nextrune(-1);
	} else { /* move cursor to the end of the word */
		while (text[cursor] && strchr(worddelimiters, text[cursor]))
			cursor = nextrune(+1);
		while (text[cursor] && !strchr(worddelimiters, text[cursor]))
			cursor = nextrune(+1);
	}
}

static void keyrelease(XKeyEvent *ev) {
	char buf[32];
	int len;
	KeySym ksym;
	Status status;
	if (!alttab)
		return;
	if (tabbed) {
		tabbed = 0;
		return;
	}

	if (ev->state & Mod1Mask) {
		if (ev->state & ShiftMask)
			return;
		if (sel->text && sel->text[0] == '>')
			return;
		puts((sel && !(ev->state & ShiftMask)) ? sel->text : text);
		if (!(ev->state & ControlMask)) {
			cleanup();
			exit(0);
		}
		if (sel)
			sel->out = 1;
	}

}

double easeOutQuint( double t ) {
    return 1 + (--t) * t * t;
}

void animatesel() {
	if (!animated || !framecount)
		return;
	int time;
	time  = 0;
	drw_setscheme(drw, scheme[SchemeSel]);
	while (time < framecount)
	{
		// bottom animation
		if (sely + lineheight < mh - 10)
			drw_rect(drw, 0, sely + (lineheight - 4), mw, (easeOutQuint(((double)time/framecount)) * (mh - (lineheight - 4) - sely)), 1, 1, 0);
		// top animation
		drw_rect(drw, 0, sely + 4 - (easeOutQuint(((double)time/framecount)) * (sely + 4)), mw, (easeOutQuint(((double)time/framecount)) * sely), 1, 1, 0);
		drw_map(drw, win, 0, 0, mw, mh);
		time++;
		usleep(19000);
	}
}

void spawn(char *cmd) {
	char command[1000];
	strcpy(command, cmd);
	strcat(command, " &> /dev/null");
	system(command);
	exit(0);
}

void animaterect(int x1, int y1, int w1, int h1, int x2, int y2, int w2, int h2) {
	if (!animated || !framecount)
		return;
	int time;
	time  = 0;
	double timefactor = 0;
	drw_setscheme(drw, scheme[SchemeSel]);
	while (time < framecount)
	{
		timefactor = easeOutQuint((double)time/framecount);
		drw_rect(drw, x1 + (x2 - x1) * timefactor, y1 + (y2 - y1) * timefactor, w1 + (w2 - w1) * timefactor, h1 + (h2 - h1) * timefactor, 1, 1, 0);
		drw_map(drw, win, 0, 0, mw, mh);
		time++;
		usleep(19000);
	}
}

static void
keypress(XKeyEvent *ev)
{
	char buf[32];
	int len;
	KeySym ksym;
	Status status;

	len = XmbLookupString(xic, ev, buf, sizeof buf, &ksym, &status);
	switch (status) {
	default: /* XLookupNone, XBufferOverflow */
		return;
	case XLookupChars:
		goto insert;
	case XLookupKeySym:
	case XLookupBoth:
		break;
	}

	if (ev->state & ControlMask) {
		switch(ksym) {
		case XK_a: ksym = XK_Home;      break;
		case XK_b: ksym = XK_Left;      break;
		case XK_c: ksym = XK_Escape;    break;
		case XK_d: ksym = XK_Delete;    break;
		case XK_e: ksym = XK_End;       break;
		case XK_f: ksym = XK_Right;     break;
		case XK_g: ksym = XK_Escape;    break;
		case XK_h: ksym = XK_BackSpace; break;
		case XK_i: ksym = XK_Tab;       break;
		case XK_j: /* fallthrough */
		case XK_J: /* fallthrough */
		case XK_m: /* fallthrough */
		case XK_M: ksym = XK_Return; ev->state &= ~ControlMask; break;
		case XK_n: ksym = XK_Down;      break;
		case XK_p: ksym = XK_Up;        break;

		case XK_v: /* paste clipboard */
            XConvertSelection(dpy, (ev->state & ShiftMask) ? clip : XA_PRIMARY,
                              utf8, utf8, win, CurrentTime);
            drawmenu();
            return;
			break;

		case XK_k: /* delete right */
			text[cursor] = '\0';
			match();
			break;
		case XK_u: /* delete left */
			insert(NULL, 0 - cursor);
			break;
		case XK_w: /* delete word */
			while (cursor > 0 && strchr(worddelimiters, text[nextrune(-1)]))
				insert(NULL, nextrune(-1) - cursor);
			while (cursor > 0 && !strchr(worddelimiters, text[nextrune(-1)]))
				insert(NULL, nextrune(-1) - cursor);
			break;
		case XK_y: /* paste selection */
		case XK_Y:
			XConvertSelection(dpy, (ev->state & ShiftMask) ? clip : XA_PRIMARY,
			                  utf8, utf8, win, CurrentTime);
			return;
		case XK_Left:
			movewordedge(-1);
			goto draw;
		case XK_Right:
			movewordedge(+1);
			goto draw;
		case XK_Return:
		case XK_KP_Enter:
			break;
		case XK_bracketleft:
			cleanup();
			exit(1);
		default:
			return;
		}
	} else if (ev->state & ShiftMask) {
		if (alttab) {
            if (sel) {
                if (sel == items) {
                    struct item *lastitem;
                    for (lastitem = items; lastitem && lastitem->right; lastitem = lastitem->right);
                    sel = lastitem;
                    //curr = lastitem;
                    calcoffsets();
                } else {
                    if (sel->left && (sel = sel->left)->right == curr) {
                        curr = prev;
                        calcoffsets();
                    }
                }
            }
		}
	} else if (ev->state & Mod1Mask) {
		switch(ksym) {
        case XK_F4:
            cleanup();
            exit(1);
        break;
		case XK_b:
			movewordedge(-1);
			goto draw;
		case XK_f:
			movewordedge(+1);
			goto draw;
		case XK_g: ksym = XK_Home;  break;
		case XK_G: ksym = XK_End;   break;
		case XK_h: ksym = XK_Up;    break;
		case XK_j: ksym = XK_Next;  break;
		case XK_k: ksym = XK_Prior; break;
		case XK_l: ksym = XK_Down;  break;
		case XK_space:
		
		if (alttab) {
			tabbed = 0;
			alttab = 0;
		}
		break;
		case XK_Tab:
		tabbed = 1;

        if (sel) {

            struct item *lastitem;
            for (lastitem = items; lastitem && lastitem->right; lastitem = lastitem->right);

            if (sel == lastitem) {
                sel = items;
                curr = items;
			    calcoffsets();
            } else {
                if (sel->right && (sel = sel->right) == next) {
                    curr = next;
                    calcoffsets();
                }
            }
        }

		break;
		default:
			return;
		}
    } else if (ev->state & Mod4Mask) {
        if (ksym == XK_q) {
            cleanup();
            exit(1);
        }
    }

	switch(ksym) {
	default:
insert:
		if (!iscntrl(*buf))
			insert(buf, len);
		break;
	case XK_Delete:
		if (text[cursor] == '\0')
			return;
		cursor = nextrune(+1);
		/* fallthrough */
	case XK_BackSpace:
		if (cursor == 0)
			return;
		insert(NULL, nextrune(-1) - cursor);
		break;
	case XK_End:
		if (text[cursor] != '\0') {
			cursor = strlen(text);
			break;
		}
		if (next) {
			/* jump to end of list and position items in reverse */
			curr = matchend;
			calcoffsets();
			curr = prev;
			calcoffsets();
			while (next && (curr = curr->right))
				calcoffsets();
		}
		sel = matchend;
		break;
	case XK_Escape:
		cleanup();
		exit(1);
	case XK_Home:
		if (sel == matches) {
			cursor = 0;
			break;
		}
		sel = curr = matches;
		calcoffsets();
		break;
	case XK_Left:
		if ((ev->state & ShiftMask || ev->state & Mod4Mask) && (leftcmd || rightcmd)) {
			char *tmpcmd;
			if (leftcmd)
				tmpcmd = leftcmd;
			else
				tmpcmd = rightcmd;

			animated = 1;
			animaterect(mw, 0, 0, mh, 0, 0, mw, mh);
			cleanup();
			spawn(tmpcmd);
			break;
		}

		if (cursor > 0 && (!sel || !sel->left || lines > 0)) {
			cursor = nextrune(-1);
			break;
		}
		if (lines > 0)
			return;
		/* fallthrough */
	case XK_Up:
		if (sel && sel->left && (sel = sel->left)->right == curr) {
			curr = prev;
			calcoffsets();
		}
		break;
	case XK_Next:
		if (!next)
			return;
		sel = curr = next;
		calcoffsets();
		break;
	case XK_Prior:
		if (!prev)
			return;
		sel = curr = prev;
		calcoffsets();
		break;
	case XK_Return:
	case XK_KP_Enter:
		if (sel && sel->text[0] == '>')
			break;
		animatesel();

		puts((sel && !(ev->state & ShiftMask)) ? sel->text : text);
		if (!(ev->state & ControlMask)) {
			cleanup();
			exit(0);
		}
		if (sel)
			sel->out = 1;
		break;
	case XK_Right:
		if ((ev->state & ShiftMask || ev->state & Mod4Mask) && (rightcmd || leftcmd)) {
			char *tmpcmd;
			if (rightcmd)
				tmpcmd = rightcmd;
			else
				tmpcmd = leftcmd;

			animated = 1;
			animaterect(0, 0, 0, mh, 0, 0, mw, mh);
			cleanup();
			spawn(tmpcmd);
			break;
		}
		if (text[cursor] != '\0') {
			cursor = nextrune(+1);
			break;
		}
		if (lines > 0)
			return;
		/* fallthrough */
	case XK_Down:
		if (sel && sel->right && (sel = sel->right) == next) {
			curr = next;
			calcoffsets();
		}
		break;
	case XK_Tab:
		if (!alttab) {
			if (!sel)
				return;
			strncpy(text, sel->text, sizeof text - 1);
			text[sizeof text - 1] = '\0';
			cursor = strlen(text);
			match();
		} else {
			tabbed = 1;
		}
		break;

	}

draw:
	drawmenu();
}


static void
setselection(XEvent *e)
{
	struct item *item;
	XMotionEvent *ev = &e->xmotion;
	int x = 0, y = 0, h = bh, w;

	if (ev->window != win)
		return;

	/* right-click: exit */
	if (prompt && *prompt)
		x += promptw;

	/* input field */
	w = (lines > 0 || !matches) ? mw - x : inputw;

	/* left-click on input: clear input,
	 * NOTE: if there is no left-arrow the space for < is reserved so
	 *       add that to the input width */

	if (lines > 0) {
		/* vertical list: (ctrl)left-click on item */
		w = mw - x;
		for (item = curr; item != next; item = item->right) {
			y += h;
			if (ev->y >= y && ev->y <= (y + h)) {
				if (sel == item)
					return;
				sel = item;
				if (sel) {
					//sel->out = 1;
					drawmenu();
				}
				return;
			}
		}
	} else if (matches) {
		/* left-click on left arrow */
		x += inputw;
		w = TEXTW("<");
		/* horizontal list: (ctrl)left-click on item */
		for (item = curr; item != next; item = item->right) {
			x += w;
			w = MIN(TEXTW(item->text), mw - x - TEXTW(">"));
			if (ev->x >= x && ev->x <= x + w) {
				if (sel == item)
					return;				
				sel = item;
				if (sel) {
					//sel->out = 1;
					drawmenu();
				}
				return;
			}
		}
		/* left-click on right arrow */
		w = TEXTW(">");
		x = mw - w;
	}
}

static void
buttonpress(XEvent *e)
{
	struct item *item;
	XButtonPressedEvent *ev = &e->xbutton;
	int x = 0, y = 0, h = bh, w;

	if (ev->window != win)
		return;

	/* right-click: exit */
	if (ev->button == Button3)
		exit(1);

	if (prompt && *prompt)
		x += promptw;

	/* input field */
	w = (lines > 0 || !matches) ? mw - x : inputw;

	/* left-click on input: clear input,
	 * NOTE: if there is no left-arrow the space for < is reserved so
	 *       add that to the input width */
	if (ev->button == Button1) {
		if ((lines <= 0 && ev->x >= 0 && ev->x <= x + w +
		((!prev || !curr->left) ? TEXTW("<") : 0)) ||
		(lines > 0 && ev->y >= y && ev->y <= y + h)) {
			insert(NULL, -cursor);
			drawmenu();
			return;
		} else {
			if (lines > 0) {
				/* vertical list: (ctrl)left-click on item */
				w = mw - x;
				item = sel;
				if (sel && sel->text[0] == '>')
					return;
				animatesel();
				puts(item->text);
				if (!(ev->state & ControlMask))
					exit(0);
				sel = item;
				if (sel) {
					sel->out = 1;
					drawmenu();
				}
				return;
			} else if (matches) {
				/* left-click on left arrow */
				x += inputw;
				w = TEXTW("<");
				if (prev && curr->left) {
					if (ev->x >= x && ev->x <= x + w) {
						sel = curr = prev;
						calcoffsets();
						drawmenu();
						return;
					}
				}
				/* horizontal list: (ctrl)left-click on item */
				for (item = curr; item != next; item = item->right) {
					x += w;
					w = MIN(TEXTW(item->text), mw - x - TEXTW(">"));
					if (ev->x >= x && ev->x <= x + w) {
						if (sel && item->text[0] == '>')
							break;
						animatesel();
						puts(item->text);
						if (!(ev->state & ControlMask))
							exit(0);
						sel = item;
						if (sel) {
							sel->out = 1;
							drawmenu();
						}
						return;
					}
				}
				/* left-click on right arrow */
				w = TEXTW(">");
				x = mw - w;
				if (next && ev->x >= x && ev->x <= x + w) {
					sel = curr = next;
					calcoffsets();
					drawmenu();
					return;
				}
			}
		}
	}

	/* middle-mouse click: paste selection */
	if (ev->button == Button2) {
		XConvertSelection(dpy, (ev->state & ShiftMask) ? clip : XA_PRIMARY,
		                  utf8, utf8, win, CurrentTime);
		drawmenu();
		return;
	}
	/* scroll up */
	if (ev->button == Button4 && prev) {
		sel = curr = prev;
		calcoffsets();
		drawmenu();
		return;
	}
	/* scroll down */
	if (ev->button == Button5 && next) {
		sel = curr = next;
		calcoffsets();
		drawmenu();
		return;
	}
}

static void
paste(void)
{
	char *p, *q;
	int di;
	unsigned long dl;
	Atom da;

	/* we have been given the current selection, now insert it into input */
	if (XGetWindowProperty(dpy, win, utf8, 0, (sizeof text / 4) + 1, False,
	                   utf8, &da, &di, &dl, &dl, (unsigned char **)&p)
	    == Success && p) {
		insert(p, (q = strchr(p, '\n')) ? q - p : (ssize_t)strlen(p));
		XFree(p);
	}
	drawmenu();
}

static void
readstdin(void)
{
	char buf[sizeof text], *p;
	size_t i, imax = 0, size = 0;
	unsigned int tmpmax = 0;

 	if(passwd || inputonly){
 	  inputw = lines = 0;
 	  return;
 	}

	/* read each line from stdin and add it to the item list */
	for (i = 0; fgets(buf, sizeof buf, stdin); i++) {
		if (i + 1 >= size / sizeof *items)
			if (!(items = realloc(items, (size += BUFSIZ))))
				die("cannot realloc %u bytes:", size);
		if ((p = strchr(buf, '\n')))
			*p = '\0';
		if (!(items[i].text = strdup(buf)))
			die("cannot strdup %u bytes:", strlen(buf) + 1);
		items[i].out = 0;
		drw_font_getexts(drw->fonts, buf, strlen(buf), &tmpmax, NULL);
		if (tmpmax > inputw) {
			inputw = tmpmax;
			imax = i;
		}
	}
	if (items)
		items[i].text = NULL;
	inputw = items ? TEXTW(items[imax].text) : 0;
	lines = MIN(lines, i);
}

static void
run(void)
{
	XEvent ev;
	Time lasttime = 0;
	int i;

	if (toast) {
		drawmenu();
		usleep(toast * 100000);
		exit(0);
	}

	while (!XNextEvent(dpy, &ev)) {
		if (preselected) {
			for (i = 0; i < preselected; i++) {
				if (sel && sel->right && (sel = sel->right) == next) {
					curr = next;
					calcoffsets();
				}
			}
			drawmenu();
			preselected = 0;
		}

		if (XFilterEvent(&ev, win))
			continue;
		switch(ev.type) {
		case MotionNotify:
			if ((ev.xmotion.time - lasttime) <= (1000 / 60))
				continue;
			lasttime = ev.xmotion.time;
			setselection(&ev);
			break;
		case DestroyNotify:
			if (ev.xdestroywindow.window != win)
				break;
			cleanup();
			exit(1);
		case ButtonPress:
			buttonpress(&ev);
			break;
		case Expose:
			if (ev.xexpose.count == 0)
				drw_map(drw, win, 0, 0, mw, mh);
			break;
		case FocusIn:
			/* regrab focus from parent window */
			if (ev.xfocus.window != win)
				grabfocus();
			break;
		case KeyPress:
			keypress(&ev.xkey);
			break;
		case KeyRelease:
			keyrelease(&ev.xkey);
			break;
		case SelectionNotify:
			if (ev.xselection.property == utf8)
				paste();
			break;
		case VisibilityNotify:
			if (ev.xvisibility.state != VisibilityUnobscured)
				XRaiseWindow(dpy, win);
			break;
		}
	}
}

static void
setup(void)
{
	int x, y, i, j;
	unsigned int du;
	XSetWindowAttributes swa;
	XIM xim;
	Window w, dw, *dws;
	XWindowAttributes wa;

	char wmclass[20];

	if (!managed)
		strcpy(wmclass, "dmenu");
	else
		strcpy(wmclass, "floatmenu");

	XClassHint ch = {wmclass, wmclass};

#ifdef XINERAMA
	XineramaScreenInfo *info;
	Window pw;
	int a, di, n, area = 0;
#endif
	/* init appearance */
	for (j = 0; j < SchemeLast; j++)
		scheme[j] = drw_scm_create(drw, colors[j], 3);

	clip = XInternAtom(dpy, "CLIPBOARD",   False);
	utf8 = XInternAtom(dpy, "UTF8_STRING", False);

	/* calculate menu geometry */
	bh = drw->fonts->h + 12;
	bh = MAX(bh,lineheight);	/* make a menu line AT LEAST 'lineheight' tall */

	lines = MAX(lines, 0);
	mh = (lines + 1) * bh;
	promptw = commented ? bh * 15 : (prompt && *prompt) ? TEXTW(prompt) - lrpad / 4 : 0;

#ifdef XINERAMA
	i = 0;
	if (parentwin == root && (info = XineramaQueryScreens(dpy, &n))) {
		XGetInputFocus(dpy, &w, &di);
		if (mon >= 0 && mon < n)
			i = mon;
		else if (w != root && w != PointerRoot && w != None) {
			/* find top-level window containing current input focus */
			do {
				if (XQueryTree(dpy, (pw = w), &dw, &w, &dws, &du) && dws)
					XFree(dws);
			} while (w != root && w != pw);
			/* find xinerama screen with which the window intersects most */
			if (XGetWindowAttributes(dpy, pw, &wa))
				for (j = 0; j < n; j++)
					if ((a = INTERSECT(wa.x, wa.y, wa.width, wa.height, info[j])) > area) {
						area = a;
						i = j;
					}
		}
		/* no focused window is on screen, so use pointer location instead */
		if (mon < 0 && !area && XQueryPointer(dpy, root, &dw, &dw, &x, &y, &di, &di, &du))
			for (i = 0; i < n; i++)
				if (INTERSECT(x, y, 1, 1, info[i]))
					break;
		if (centered) {
			if (dmw && dmw < info[i].width && info[i].width)
				mw = dmw;
			else
				mw = info[i].width - 100;
			
			while ((lines + 1) * bh > info[i].height)
			{
				lines--;
			}
			
			mh = (lines + 1) * bh;
			x = info[i].x_org + ((info[i].width  - mw) / 2);
			y = info[i].y_org + ((info[i].height - mh) / 2);

			if (y < 0)
				y = 0;

		} else if (followcursor){
			if (dmw)
				mw = dmw;
			else
				mw = MIN(MAX(max_textw() + promptw, min_width), wa.width);
			getrootptr(&x, &y);
			if (x > info[i].x_org + (drw->w - info[i].x_org) / 2) {
				x = x - mw + 20;
			} else {
				x = x - 20;
			}
			if (y > info[i].y_org + (drw->h - info[i].y_org) / 2) {
				y = y - mh + 20;
			} else {
				y = y - 20;
			}

			if (x < 0)
				x = 0;
			if (y < 0)
				y = 0;

		} else {
			if (dmy <= -1)
				dmy = drw->fonts->h * 1.55;
			mw = ((dmw>0 && dmw < info[i].width) ? dmw : info[i].width);
			x = rightxoffset ? info[i].x_org + info[i].width - dmx - mw - 2 * border_width : info[i].x_org + dmx;
			y = info[i].y_org + (topbar ? dmy : info[i].height - mh - dmy);
		}

		if (mh > drw->h - 10) {
			mh = drw->h - border_width * 2 - 10;
			fprintf(stderr, "lineheight %d", lineheight);
			lines = (drw->h / (lineheight ? lineheight : bh)) - 1; 
		}

		if (mw > drw->w - 10) {
			mw = drw->w - border_width * 2;
		}

		if (x < info[i].x_org)
			x = info[i].x_org;
		if (x + mw > info[i].x_org + info[i].width)
			x = info[i].x_org + info[i].width - mw - border_width*2;
		if (fullheight) {
		        y = info[i].y_org + 32;
		        mh = drw->h - border_width * 2 - (drw->h - info[i].height + 32);
		        lines = (drw->h / lineheight) - 2;
		} else {
			if (y + mh > drw->h)
				y = drw->h - mh;
		}
		XFree(info);
	} else
#endif
	{
		if (!XGetWindowAttributes(dpy, parentwin, &wa))
			die("could not get embedding window attributes: 0x%lx",
			    parentwin);
		if (centered) {
			mw = MIN(MAX(max_textw() + promptw, min_width), wa.width);
			x = (wa.width  - mw) / 2;
			y = (wa.height - mh) / 2;
		} else if (followcursor){
			getrootptr(&x, &y);
			if (x > drw->w / 2) {
				x = x - mw;
			}
			if (y > drw->h / 2) {
				y = y - mh;
			}
			mw = MIN(MAX(max_textw() + promptw, min_width), wa.width);
		} else {
			x = dmx;
			y = topbar ? dmy : wa.height - mh - dmy;
			mw = ((dmw>0 && dmw < wa.width) ? dmw : wa.width);
		}
	}

	inputw = MIN(inputw, mw/3);
	match();

	/* create menu window */
	swa.override_redirect = managed ? False : True;
	swa.background_pixel = scheme[SchemeNorm][ColBg].pixel;
	swa.event_mask = ExposureMask | KeyPressMask | KeyReleaseMask | VisibilityChangeMask |
	                 ButtonPressMask | PointerMotionMask;;
	win = XCreateWindow(dpy, root, x, y, mw, mh, border_width,
	                    DefaultDepth(dpy, screen), CopyFromParent,
	                    DefaultVisual(dpy, screen),
	                    CWOverrideRedirect | CWBackPixel | CWEventMask, &swa);
	XSetWindowBorder(dpy, win, scheme[SchemeSel][ColBg].pixel);

	XSetClassHint(dpy, win, &ch);

	if (managed)
		XStoreName(dpy, win, searchtext ? searchtext : "menu");

	/* input methods */
	if ((xim = XOpenIM(dpy, NULL, NULL, NULL)) == NULL)
		die("XOpenIM failed: could not open input device");

	xic = XCreateIC(xim, XNInputStyle, XIMPreeditNothing | XIMStatusNothing,
	                XNClientWindow, win, XNFocusWindow, win, NULL);

	XMapRaised(dpy, win);
	if (embed) {
		XSelectInput(dpy, parentwin, FocusChangeMask | SubstructureNotifyMask);
		if (XQueryTree(dpy, parentwin, &dw, &w, &dws, &du) && dws) {
			for (i = 0; i < du && dws[i] != win; ++i)
				XSelectInput(dpy, dws[i], FocusChangeMask);
			XFree(dws);
		}
		grabfocus();
	}
	drw_resize(drw, mw, mh);
	drawmenu();
}

static void
usage(void)
{
	fputs("usage: instantmenu [-bfinPv] [-l lines] [-p prompt] [-fn font] [-m monitor]\n"
	      "             [-x xoffset] [-xr right xoffset] [-y yoffset] [-w width]\n"
	      "             [-h height]\n"

	      "             [-nb color] [-nf color] [-sb color] [-sf color] [-w windowid]\n", stderr);
	exit(1);
}

int
main(int argc, char *argv[])
{
	XWindowAttributes wa;
	int i, fast = 0;

	for (i = 1; i < argc; i++)
		/* these options take no arguments */
		if (!strcmp(argv[i], "-v")) {      /* prints version information */
			puts("instantmenu-"VERSION);
			exit(0);
		} else if (!strcmp(argv[i], "-b")) /* appears at the bottom of the screen */
			topbar = 0;
		else if (!strcmp(argv[i], "-f"))   /* grabs keyboard before reading stdin */
			fast = 1;
		else if (!strcmp(argv[i], "-T"))   /* grabs keyboard before reading stdin */
			toast = atoi(argv[++i]);
		else if (!strcmp(argv[i], "-ct")) {   /* centers dmenu on screen */
			commented = 1;
			static char commentprompt[200];
			prompt = commentprompt + 1;
			strcpy(prompt, "prompts");
		} else if (!strcmp(argv[i], "-c"))   /* centers dmenu on screen */
			centered = 1;
		else if (!strcmp(argv[i], "-C"))   /* go to mouse position */
			followcursor = 1;
		else if (!strcmp(argv[i], "-S"))   /* confirm using the space key */
			spaceconfirm = 1;
		else if (!strcmp(argv[i], "-I"))   /* input only */
			inputonly = 1;
        else if (!strcmp(argv[i], "-s")) {
            /* enable smart case */
            smartcase = 1;
			fstrncmp = strncasecmp;
			fstrstr = cistrstr;
        }
		else if (!strcmp(argv[i], "-F"))   /* disables fuzzy matching */
			/* disables fuzzy matching */
			fuzzy = 0;
        else if (!strcmp(argv[i], "-E")) {
			/* enabled exact matching */
			exact = 1;
            fuzzy = 0;
        } else if (!strcmp(argv[i], "-H")) {
			fullheight = 1;
		}
		else if (!strcmp(argv[i], "-i")) { /* case-insensitive item matching */
			fstrncmp = strncasecmp;
			fstrstr = cistrstr;
		} else if (!strcmp(argv[i], "-n")) { /* instant select only match */
			instant = 1;
		} else if (!strcmp(argv[i], "-P"))   /* is the input a password */
			passwd = 1;
		else if (!strcmp(argv[i], "-G"))   /* don't grab the keyboard */
			nograb = 1;
		else if (!strcmp(argv[i], "-A"))   /* alt-tab behaviour */
			alttab = 1;
		else if (!strcmp(argv[i], "-wm"))/* display as managed wm window */
			managed = 1;
		else if (i + 1 == argc)
			usage();
		else if (!strcmp(argv[i], "-rc"))   /* adds prompt to left of input field */
			rightcmd = argv[++i];
		else if (!strcmp(argv[i], "-lc"))   /* adds prompt to left of input field */
			leftcmd = argv[++i];
		/* these options take one argument */
		else if (!strcmp(argv[i], "-l"))   /* number of lines in vertical list */
			lines = atoi(argv[++i]);
		else if (!strcmp(argv[i], "-x"))   /* window x offset */
			dmx = atoi(argv[++i]);
		else if (!strcmp(argv[i], "-xr")) {/* window x offset from the right */
			rightxoffset = 1;
			dmx = atoi(argv[++i]);
		} else if (!strcmp(argv[i], "-y"))   /* window y offset (from bottom up if -b) */
			dmy = atoi(argv[++i]);
		else if (!strcmp(argv[i], "-w"))   /* make instantmenu this wide */
			dmw = atoi(argv[++i]);
		else if (!strcmp(argv[i], "-m")) // select monitor
			mon = atoi(argv[++i]);
		else if (!strcmp(argv[i], "-p"))   /* adds prompt to left of input field */
			prompt = argv[++i];
		else if (!strcmp(argv[i], "-q"))   /* adds prompt to left of input field */
			searchtext = argv[++i];
		else if (!strcmp(argv[i], "-fn"))  /* font or font set */
			fonts[0] = argv[++i];
		else if(!strcmp(argv[i], "-h")) { /* minimum height of one menu line */
			if (!fullheight) {
				lineheight = atoi(argv[++i]);
				lineheight = MAX(lineheight,8); /* reasonable default in case of value too small/negative */
			}
		} else if(!strcmp(argv[i], "-a")) /* animation duration */
			framecount = atoi(argv[++i]);
		else if (!strcmp(argv[i], "-nb"))  /* normal background color */
			colors[SchemeNorm][ColBg] = argv[++i];
		else if (!strcmp(argv[i], "-nf"))  /* normal foreground color */
			colors[SchemeNorm][ColFg] = argv[++i];
		else if (!strcmp(argv[i], "-sb"))  /* selected background color */
			colors[SchemeSel][ColBg] = argv[++i];
		else if (!strcmp(argv[i], "-sf"))  /* selected foreground color */
			colors[SchemeSel][ColFg] = argv[++i];
		else if (!strcmp(argv[i], "-W"))   /* embedding window id */
			embed = argv[++i];
		else if (!strcmp(argv[i], "-bw"))
			border_width = atoi(argv[++i]); /* border width */
        else if (!strcmp(argv[i], "-ps")) {
            /* preselected item */
            if (*argv[i + 1] == '-') {
			    preselected = atoi(argv[++i] + 1);
            } else {
			    preselected = atoi(argv[++i]);
            }
        }
		else if (!strcmp(argv[i], "-it")) {   /* embedding window id */
			const char * text = argv[++i];
			insert(text, strlen(text));
		} else
			usage();

	if (!setlocale(LC_CTYPE, "") || !XSupportsLocale())
		fputs("warning: no locale support\n", stderr);
	if (!(dpy = XOpenDisplay(NULL)))
		die("cannot open display");
	screen = DefaultScreen(dpy);
	root = RootWindow(dpy, screen);
	if (!embed || !(parentwin = strtol(embed, NULL, 0)))
		parentwin = root;
	if (!XGetWindowAttributes(dpy, parentwin, &wa))
		die("could not get embedding window attributes: 0x%lx",
		    parentwin);
	drw = drw_create(dpy, screen, root, wa.width, wa.height);
	if (!drw_fontset_create(drw, fonts, LENGTH(fonts)))
		die("no fonts could be loaded.");
	lrpad = drw->fonts->h;

	if (fullheight || lineheight == -1)
		lineheight = drw->fonts->h*2.5;

	if (prompt && dmw && TEXTW(prompt) + 100 > dmw && dmw < mw - 300)
		dmw += TEXTW(prompt);

#ifdef __OpenBSD__
	if (pledge("stdio rpath", NULL) == -1)
		die("pledge");
#endif

	if (fast && !isatty(0)) {
		grabkeyboard();
		readstdin();
	} else {
		readstdin();
		grabkeyboard();
	}

	if (dmw <= -1) {
		int maxw = max_textw() * 1.3 + (prompt ? TEXTW(prompt) : 0);
		if (dmw * (-1) > maxw) {
			dmw = dmw * (-1);
		} else {
			dmw = maxw;
		}

	}
	setup();
	run();

	return 1; /* unreachable */
}
