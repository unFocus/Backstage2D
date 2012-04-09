/*
========================================================================
=      ================================  =====  ==================  ====
=  ===  ===============================   ===   ==================  ====
=  ====  ==============================  =   =  ==================  ====
=  ===  ===  =  ==  = ===  = ===  =  ==  == ==  ===   ===  =   ===  =  =
=      ====  =  ==     ==     ==  =  ==  =====  ==  =  ==    =  ==    ==
=  ===  ===  =  ==  =  ==  =  ===    ==  =====  =====  ==  =======   ===
=  ====  ==  =  ==  =  ==  =  =====  ==  =====  ===    ==  =======    ==
=  ===  ===  =  ==  =  ==  =  ==  =  ==  =====  ==  =  ==  =======  =  =
=      =====    ==  =  ==  =  ===   ===  =====  ===    ==  =======  =  =
========================================================================


* Copyright (c) 2012 Julian Wixson / Aaron Charbonneau - Adobe Systems
*
* Special thanks to Iain Lobb - iainlobb@googlemail.com for the original BunnyMark:
*
* http://blog.iainlobb.com/2010/11/display-list-vs-blitting-results.html 
*
* This program is distributed under the terms of the MIT License as found 
* in a file called LICENSE. If it is not present, the license
* is always available at http://www.opensource.org/licenses/mit-license.php.
*
* This program is distributed in the hope that it will be useful, but
* without any waranty; without even the implied warranty of merchantability
* or fitness for a particular purpose. See the MIT License for full details.
*/

package bunnymark
{
	import flash.events.Event;
	import flash.events.TimerEvent;
	import flash.text.TextField;
	import flash.text.TextFormat;
	import flash.utils.getTimer;
	
	public class FPS extends TextField
	{
		private var frameCount:int = 0;
		private var timer:int;
		private var ms_prev:int;
		private var lastfps : Number = 60; 
		
		public function FPS(inX:Number=10.0, inY:Number=10.0, inCol:int = 0x000000)
		{
			super();
			x = inX;
			y = inY;
			selectable = false;
			defaultTextFormat = new TextFormat("_sans", 20, 0, true);
			text = "FPS:";
			textColor = inCol;
			this.addEventListener(Event.ADDED_TO_STAGE, onAddedHandler);
			
		}
		public function onAddedHandler(e:Event):void {
			stage.addEventListener(Event.ENTER_FRAME, onEnterFrame);
		}
		
		private function onEnterFrame(evt:Event):void
		{
			
			timer = getTimer();
			
			if( timer - 1000 > ms_prev )
			{
				lastfps = Math.round(frameCount/(timer-ms_prev)*1000);
				ms_prev = timer;
				text = "FPS:\n" + lastfps + "/" + stage.frameRate;
				frameCount = 0;
			}
			frameCount++;
				
		}
			
	}

}