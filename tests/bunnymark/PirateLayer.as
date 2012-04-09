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
	import backstage2d.display.Actor;
	import backstage2d.display.Image;
	import backstage2d.display.Layer;
	import flash.display.Bitmap;
	import flash.display3D.*;
	import flash.geom.Point;
	import flash.geom.Rectangle;
	
	public class PirateLayer
	{
		[Embed(source="../../assets/pirate.png")]
		private var PirateImage : Class;
		
		private var pirateHalfWidth:int;
		private var pirateHalfHeight:int;
		
		private var _pirateSpriteID:uint;
		private var maxX:int;
		private var minX:int;
		private var maxY:int;
		private var minY:int;
		
		private var pirate:Image;
		private var layer:Layer;
		
		public function PirateLayer(view:Rectangle)
		{
			setPosition(view);
		}
		public function setPosition(view:Rectangle):void {
			maxX = view.width;
			minX = view.x;
			maxY = view.height;
			minY = view.y;
		}
		
		public function createRenderLayer() : Layer
		{
			pirate = new Image( (new PirateImage() as Bitmap).bitmapData );
			
			layer = new Layer();
			layer.add( pirate );
			
			pirateHalfHeight = pirate.height / 2;
			pirateHalfWidth = pirate.width / 2;
			
			return layer;
		}
		
		public function update(currentTime:Number) : void
		{		
			pirate.x = (maxX - (pirateHalfWidth)) * (0.5 + 0.5 * Math.sin(currentTime / 3000));
			pirate.y = (maxY - (pirateHalfHeight) + 70 - 30 * Math.sin(currentTime / 100));
		}
	}
}